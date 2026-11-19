use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("cannot read archive: {0}")]
    BadArchive(String),
    #[error("archive contains an unsafe path: {0}")]
    UnsafePath(String),
    #[error("archive contains a forbidden file type (mods may only contain assets): {0}")]
    ForbiddenFile(String),
    #[error("the archive is password-protected, which is not supported")]
    Encrypted,
    #[error("archive has too many entries ({0})")]
    TooManyEntries(usize),
    #[error("archive exceeds the size limit")]
    TooLarge,
    #[error("archive contains no .vpk files")]
    NoVpk,
    #[error("invalid VPK '{0}': {1}")]
    BadVpk(String, String),
    #[error("VPK '{0}' depends on external chunk files (_000.vpk), which are not supported")]
    ExternalChunks(String),
    #[error("gameinfo.gi has no SearchPaths block or no 'Game citadel' line; refusing to edit it")]
    GameinfoUnrecognized,
    #[error("target already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("this mod is already installed as '{0}'")]
    AlreadyInstalled(String),
    #[error("too many mods: only {0} load slots are available")]
    TooManySlots(usize),
    #[error("path escapes the allowed directory: {0}")]
    OutsideRoot(PathBuf),
    #[error("journal error: {0}")]
    Journal(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
