use crate::FilePathDescription;
use crate::platform::unix::description_path_unix;

pub type FilePathDescriberImpl = LinuxImpl;

pub const OS_NAME: &'static str = "linux";

pub struct LinuxImpl {
}

impl FilePathDescription for LinuxImpl {
    fn description(&self) -> String {
        return description_path_unix();
    }
}

#[path = "./maclinuxshared.rs"]
mod unix;