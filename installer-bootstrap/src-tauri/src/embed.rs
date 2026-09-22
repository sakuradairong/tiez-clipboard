/// Bytes of the inner NSIS setup, present only when `TIEZ_SETUP_EXE` pointed
/// at a real executable during this build. A dev or Linux build leaves this
/// empty on purpose; the wizard then reports `setup_not_embedded` instead of
/// pretending an install ran.
pub fn embedded_setup_bytes() -> Option<&'static [u8]> {
    #[cfg(embedded_setup)]
    {
        Some(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/embedded-setup.exe"
        )))
    }
    #[cfg(not(embedded_setup))]
    {
        None
    }
}
