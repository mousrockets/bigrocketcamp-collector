//! Windows plumbing: registry, DPAPI, start with Windows, one instance, and a process that stays out of the way.

pub mod ui;

use crate::service::Vault;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::{fs, io, ptr, slice};
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, CreateMutexW, GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetPriorityClass,
    SetProcessInformation,
};

pub const APP_ID: &str = "BigRocketCampCollector";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn registry_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let mut size = 0u32;
    // SAFETY: the strings are NUL-terminated and outlive the calls; the buffer holds `size` bytes.
    unsafe {
        let found =
            RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, ptr::null_mut(), ptr::null_mut(), &mut size);
        if found != ERROR_SUCCESS || size < 2 {
            return None;
        }
        let mut buffer = vec![0u16; size as usize / 2];
        let data = buffer.as_mut_ptr().cast::<c_void>();
        if RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, ptr::null_mut(), data, &mut size)
            != ERROR_SUCCESS
        {
            return None;
        }
        let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    }
}

fn set_registry_string(root: HKEY, key: &str, name: &str, value: &str) -> bool {
    let (key, name, value) = (wide(key), wide(name), wide(value));
    let bytes = (value.len() * 2) as u32;
    // SAFETY: NUL-terminated strings that outlive the call; `bytes` is the size of `value`.
    unsafe { RegSetKeyValueW(root, key.as_ptr(), name.as_ptr(), REG_SZ, value.as_ptr().cast(), bytes) == ERROR_SUCCESS }
}

fn delete_registry_value(root: HKEY, key: &str, name: &str) {
    let (key, name) = (wide(key), wide(name));
    // SAFETY: NUL-terminated strings that outlive the call.
    unsafe { RegDeleteKeyValueW(root, key.as_ptr(), name.as_ptr()) };
}

pub fn steam_folder() -> Option<PathBuf> {
    let path = registry_string(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath")?;
    Some(PathBuf::from(path.replace('/', r"\")))
}

pub fn starts_with_windows() -> bool {
    registry_string(HKEY_CURRENT_USER, RUN_KEY, APP_ID).is_some()
}

/// Started by Windows, the collector goes straight to the notification area when everything is set.
pub fn set_start_with_windows(enabled: bool) {
    match std::env::current_exe() {
        Ok(exe) if enabled => {
            set_registry_string(HKEY_CURRENT_USER, RUN_KEY, APP_ID, &format!("\"{}\" --background", exe.display()));
        }
        _ => delete_registry_value(HKEY_CURRENT_USER, RUN_KEY, APP_ID),
    }
}

/// False when the collector already runs for this Windows user.
pub fn first_instance() -> bool {
    let name = wide(&format!(r"Local\{APP_ID}"));
    // SAFETY: the name is NUL-terminated; the mutex is kept open for the life of the process on purpose.
    unsafe {
        let mutex = CreateMutexW(ptr::null(), 0, name.as_ptr());
        !mutex.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Below normal priority and Windows' efficiency mode: the game always comes first.
pub fn stay_out_of_the_way() {
    let throttling = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    // SAFETY: the pseudo handle of this process, and a structure of the size given.
    unsafe {
        let process = GetCurrentProcess();
        SetPriorityClass(process, BELOW_NORMAL_PRIORITY_CLASS);
        SetProcessInformation(
            process,
            ProcessPowerThrottling,
            (&raw const throttling).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// Encrypts or decrypts with DPAPI: only this Windows user, on this PC, can read the result.
fn dpapi(data: &[u8], protect: bool) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    // SAFETY: `input` points to `data` for the call; `output` is allocated by Windows and freed below.
    unsafe {
        let done = if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as HLOCAL);
        Ok(result)
    }
}

pub struct DpapiVault {
    path: PathBuf,
}

impl DpapiVault {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Option<String> {
        let protected = fs::read(&self.path).ok()?;
        String::from_utf8(dpapi(&protected, false).ok()?).ok()
    }
}

impl Vault for DpapiVault {
    fn save(&self, token: &str) -> io::Result<()> {
        let protected = dpapi(token.as_bytes(), true)?;
        write_atomically(&self.path, &protected)
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let draft = path.with_extension("tmp");
    fs::write(&draft, bytes)?;
    fs::rename(&draft, path)
}
