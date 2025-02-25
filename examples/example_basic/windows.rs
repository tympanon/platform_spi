use crate::FilePathDescription;

pub type FilePathDescriberImpl = WindowsImpl;

pub const OS_NAME: &'static str = "windows";

pub struct WindowsImpl {
}

impl FilePathDescription for WindowsImpl {
    fn description(&self) -> String {
        return "Directories are seperated by \\, e.g. example\\file\\path".to_string();
    }
}