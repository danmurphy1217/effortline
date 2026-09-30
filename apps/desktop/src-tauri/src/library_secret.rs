//! OS secret access stays in the desktop shell. Keys never cross the IPC boundary.
use effortline_core::library::{LibrarySecret, LibrarySecretProvider, SecretUnavailable};
use std::path::Path;

pub(super) struct KeychainSecret<'a> {
    pub directory: &'a Path,
    pub service: &'a str,
    pub account: &'a str,
}

impl LibrarySecretProvider for KeychainSecret<'_> {
    fn load_secret(&self) -> Result<LibrarySecret, SecretUnavailable> {
        #[cfg(target_os = "macos")]
        {
            use security_framework::os::macos::keychain::SecKeychain;
            use zeroize::Zeroizing;

            // Security.framework OSStatus values. Only item-not-found permits creation.
            const ITEM_NOT_FOUND: i32 = -25300;
            const DUPLICATE_ITEM: i32 = -25299;
            let keychain = SecKeychain::default().map_err(|_| SecretUnavailable)?;
            let read = || keychain.find_generic_password(self.service, self.account);
            match read() {
                Ok((password, _)) => {
                    let bytes = password
                        .as_ref()
                        .try_into()
                        .map_err(|_| SecretUnavailable)?;
                    return Ok(LibrarySecret::from_bytes(bytes));
                }
                Err(error) if error.code() == ITEM_NOT_FOUND => {}
                Err(_) => return Err(SecretUnavailable),
            }
            // Never create a replacement key for an existing or partially written library.
            // A metadata failure is not evidence that the library is new.
            if self
                .directory
                .join("library.sqlite3")
                .try_exists()
                .map_err(|_| SecretUnavailable)?
                || self
                    .directory
                    .join("objects")
                    .try_exists()
                    .map_err(|_| SecretUnavailable)?
            {
                return Err(SecretUnavailable);
            }
            let mut bytes = Zeroizing::new([0_u8; 32]);
            getrandom::fill(&mut *bytes).map_err(|_| SecretUnavailable)?;
            // Add only: concurrent creators must retrieve the winning key, never overwrite it.
            match keychain.add_generic_password(self.service, self.account, bytes.as_ref()) {
                Ok(()) => {}
                Err(error) if error.code() == DUPLICATE_ITEM => {}
                Err(_) => return Err(SecretUnavailable),
            }
            let (password, _) = read().map_err(|_| SecretUnavailable)?;
            let bytes = password
                .as_ref()
                .try_into()
                .map_err(|_| SecretUnavailable)?;
            Ok(LibrarySecret::from_bytes(bytes))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (self.directory, self.service, self.account);
            Err(SecretUnavailable)
        }
    }
}
