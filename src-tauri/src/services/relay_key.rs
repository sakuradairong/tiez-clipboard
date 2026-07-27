use crate::error::{AppError, AppResult};
use chacha20poly1305::aead::rand_core::{OsRng, RngCore};
#[cfg(not(feature = "portable"))]
use std::path::Path;

#[cfg(not(feature = "portable"))]
const RELAY_KEY_SERVICE: &str = "com.tiez.clipboard-relay";
#[cfg(not(feature = "portable"))]
const RELAY_KEY_ACCOUNT: &str = "shared-key-v1";

#[cfg(not(feature = "portable"))]
fn unavailable(message: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("系统安全密钥库不可用：{}", message))
}

pub fn ensure_runtime_allowed(
    #[cfg(not(feature = "portable"))] data_dir: Option<&Path>,
    #[cfg(feature = "portable")] _data_dir: Option<&std::path::Path>,
) -> AppResult<()> {
    #[cfg(feature = "portable")]
    return Err(reject_portable());

    #[cfg(not(feature = "portable"))]
    {
        if let (Some(data_dir), Ok(executable)) = (data_dir, std::env::current_exe()) {
            if executable
                .parent()
                .is_some_and(|parent| parent.join("data") == data_dir)
            {
                return Err(AppError::Validation(
                    "便携数据目录不支持剪贴板接力安全密钥".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(any(not(feature = "portable"), test))]
fn validate_hex_key(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AppError::Validation(
            "接力共享密钥必须是 64 位小写十六进制字符串".to_string(),
        ));
    }
    Ok(value.to_string())
}

#[cfg(not(feature = "portable"))]
fn entry() -> AppResult<keyring::v1::Entry> {
    keyring::v1::Entry::new(RELAY_KEY_SERVICE, RELAY_KEY_ACCOUNT).map_err(unavailable)
}

#[cfg(feature = "portable")]
fn reject_portable() -> AppError {
    AppError::Validation("便携模式不支持剪贴板接力安全密钥".to_string())
}

pub fn load() -> AppResult<Option<[u8; 32]>> {
    #[cfg(feature = "portable")]
    return Err(reject_portable());

    #[cfg(not(feature = "portable"))]
    {
        let value = match entry()?.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(unavailable(error)),
        };
        let value = validate_hex_key(&value)?;
        let bytes = hex::decode(value)
            .map_err(|_| AppError::Validation("接力共享密钥格式无效".to_string()))?;
        let mut key = [0_u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Some(key))
    }
}

pub fn is_configured() -> AppResult<bool> {
    Ok(load()?.is_some())
}

pub fn store(
    #[cfg(not(feature = "portable"))] raw: &str,
    #[cfg(feature = "portable")] _raw: &str,
) -> AppResult<()> {
    #[cfg(feature = "portable")]
    return Err(reject_portable());

    #[cfg(not(feature = "portable"))]
    {
        let value = validate_hex_key(raw)?;
        let entry = entry()?;
        entry.set_password(&value).map_err(unavailable)?;
        let stored = entry.get_password().map_err(unavailable)?;
        if stored != value {
            return Err(unavailable("写入后校验失败"));
        }
        Ok(())
    }
}

pub fn generate() -> AppResult<String> {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let value = hex::encode(bytes);
    store(&value)?;
    Ok(value)
}

pub fn clear() -> AppResult<()> {
    #[cfg(feature = "portable")]
    return Err(reject_portable());

    #[cfg(not(feature = "portable"))]
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(unavailable(error)),
    }
}

pub fn migrate_legacy(raw: &str) -> AppResult<()> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    if is_configured()? {
        return Ok(());
    }
    store(raw)
}

pub fn validate_format(raw: &str) -> AppResult<()> {
    #[cfg(feature = "portable")]
    return Err(reject_portable());

    #[cfg(not(feature = "portable"))]
    validate_hex_key(raw).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::validate_hex_key;

    #[test]
    fn relay_key_is_strict_lowercase_hex() {
        let valid = "01".repeat(32);
        assert_eq!(validate_hex_key(&valid).unwrap(), valid);
        assert!(validate_hex_key(&"A1".repeat(32)).is_err());
        assert!(validate_hex_key(&"01".repeat(31)).is_err());
        assert!(validate_hex_key(&"gg".repeat(32)).is_err());
    }
}
