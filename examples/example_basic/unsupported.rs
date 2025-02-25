use crate::FilePathDescription;

pub type FilePathDescriberImpl = UnsupportedImpl;

pub const OS_NAME: &'static str = "unknown";
pub struct UnsupportedImpl {
}

impl FilePathDescription for UnsupportedImpl {
    fn description(&self) -> String {
        return "This platform is unknown so we do not know how file paths are written.".to_string();
    }
}