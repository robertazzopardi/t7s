/// Update pushed from a background task into the app's event loop.
pub enum BgEvent {
    /// Background action finished successfully.
    ActionDone { key: String, message: String },
    /// Background action failed.
    ActionFailed { key: String, message: String },
}
