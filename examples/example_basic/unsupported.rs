use crate::FilePathDescription;

pub type FilePathDescriberImpl = UnsupportedImpl;
pub const OS_NAME: &'static str = "unknown";
pub struct UnsupportedImpl;

impl ToString for UnsupportedImpl {
    fn to_string(&self) -> String {
        "This platform is unknown so we do not know how file paths are written.".to_string();
    }
}

//Example blanket implementation that fulfills contract
impl<T> FilePathDescription<String> for T
where T : ToString {
    fn description(&self) -> String {
        return self.to_string();
    }
}