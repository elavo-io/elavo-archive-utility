/// A progress report. `fraction` is `None` when the total size is unknown
/// and the window should pulse the bar.
#[derive(Clone, Debug)]
pub struct Progress {
    pub fraction: Option<f64>,
    pub message: String,
}

pub fn report(
    progress: &mut impl FnMut(Progress),
    fraction: Option<f64>,
    message: impl Into<String>,
) {
    progress(Progress {
        fraction,
        message: message.into(),
    });
}
