use crate::FilePathDescription;
use crate::platform::unix::description_path_unix;

pub type FilePathDescriberImpl = MacosImpl;

pub const OS_NAME: &'static str = "macos";

pub struct MacosImpl {
}

impl FilePathDescription for MacosImpl {
    fn description(&self) -> String {
        return description_path_unix();
    }
}

#[path = "./maclinuxshared.rs"]
mod unix;