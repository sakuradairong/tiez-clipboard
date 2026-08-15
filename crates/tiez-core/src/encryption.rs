//! Windows DPAPI compatibility used by every TieZ desktop runtime.

use base64::Engine;

#[cfg(windows)]
use std::ffi::c_void;

pub const ENCRYPT_PREFIX: &str = "dpapi:";

#[cfg(windows)]
type Bool = i32;
#[cfg(windows)]
type Dword = u32;

#[cfg(windows)]
#[repr(C)]
#[allow(non_snake_case)]
struct DataBlob {
    cbData: Dword,
    pbData: *mut u8,
}

#[cfg(windows)]
const CRYPTPROTECT_UI_FORBIDDEN: Dword = 0x1;

#[cfg(windows)]
#[link(name = "crypt32")]
extern "system" {
    fn CryptProtectData(
        data_in: *mut DataBlob,
        data_description: *const u16,
        optional_entropy: *mut DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: Dword,
        data_out: *mut DataBlob,
    ) -> Bool;

    fn CryptUnprotectData(
        data_in: *mut DataBlob,
        data_description: *mut *mut u16,
        optional_entropy: *mut DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: Dword,
        data_out: *mut DataBlob,
    ) -> Bool;
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[cfg(windows)]
pub fn encrypt_value(plain: &str) -> Option<String> {
    let bytes = plain.as_bytes();
    let mut input = DataBlob {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let succeeded = unsafe {
        CryptProtectData(
            &mut input,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return None;
    }

    let protected = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let encoded = base64::engine::general_purpose::STANDARD.encode(protected);
    unsafe {
        let _ = LocalFree(output.pbData.cast());
    }
    Some(format!("{ENCRYPT_PREFIX}{encoded}"))
}

#[cfg(windows)]
pub fn decrypt_value(cipher: &str) -> Option<String> {
    let payload = cipher.strip_prefix(ENCRYPT_PREFIX)?;
    let protected = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    let mut input = DataBlob {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let succeeded = unsafe {
        CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return None;
    }

    let decrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = String::from_utf8(decrypted.to_vec()).ok();
    unsafe {
        let _ = LocalFree(output.pbData.cast());
    }
    result
}

#[cfg(not(windows))]
pub fn encrypt_value(plain: &str) -> Option<String> {
    Some(plain.to_owned())
}

#[cfg(not(windows))]
pub fn decrypt_value(cipher: &str) -> Option<String> {
    (!cipher.starts_with(ENCRYPT_PREFIX)).then(|| cipher.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip_preserves_utf8() {
        let plain = "TieZ 隐私内容 🔐";

        let encrypted = encrypt_value(plain).expect("encrypt with DPAPI");

        assert!(encrypted.starts_with(ENCRYPT_PREFIX));
        assert_ne!(encrypted, plain);
        assert_eq!(decrypt_value(&encrypted).as_deref(), Some(plain));
    }

    #[cfg(windows)]
    #[test]
    fn invalid_dpapi_payload_fails_closed() {
        assert_eq!(decrypt_value("plaintext"), None);
        assert_eq!(decrypt_value("dpapi:not-base64"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_runtime_does_not_expose_dpapi_ciphertext() {
        assert_eq!(decrypt_value("dpapi:foreign"), None);
        assert_eq!(decrypt_value("plaintext").as_deref(), Some("plaintext"));
    }
}
