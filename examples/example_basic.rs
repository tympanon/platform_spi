use platform_spi::platform_spi;

/// Basic example that prints a different string depending on the built platform.
/// 
/// platform_spi attribute with the below usage
/// module_path: declares the platform implementations will be stored in the ./example_basic directory
/// target: declares that there are 3 platforms implementations - macos (see example_basic/macos.rs), windows, and linux
/// An unsupported implementation is optionally provided in example_basic/unsupported.rs
#[platform_spi(module_path="example_basic" targets = [macos, windows, linux])]
mod platform {

    //Declares a type to be implemented for each platform
    pub type FilePathDescriber = FilePathDescriberImpl;

    //Declares a constant that must be provided for each platform
    pub use OS_NAME as PLATFORM_NAME;

    //trait that each platform specific FilePathDescriberImpl must implement
    impl FilePathDescription<String> for FilePathDescriber {}

}

trait FilePathDescription<T> {
    fn description(&self) -> T;
}

fn main() {
    let os = FilePathDescriber{};
    println!("Platform is {}", PLATFORM_NAME);
    println!("In this platform file paths are written as: {}", os.description());
}