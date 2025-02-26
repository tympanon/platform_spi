use crate::platform::unix::UnixImpl;

pub type FilePathDescriberImpl = UnixImpl;
pub const OS_NAME: &'static str = "linux";

#[path = "./maclinuxshared.rs"]
mod unix;