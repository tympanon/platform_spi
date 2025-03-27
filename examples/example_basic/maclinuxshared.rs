use crate::FilePathDescription;

pub struct UnixImpl;

impl FilePathDescription<String> for UnixImpl {
    fn description(&self) -> String {
        return "Directories are seperated by /, e.g. example/file/path".to_string();
    }
}