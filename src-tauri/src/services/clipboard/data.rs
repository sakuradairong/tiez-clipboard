#[derive(Debug, Clone)]
pub enum ClipboardData {
    Text(String),
    RichText { text: String, html: String },
    Image { data_url: String },
    Files(Vec<String>),
}
