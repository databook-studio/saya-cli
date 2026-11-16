struct Reply {
    status: u16,
    chunks: Vec<&'static str>,
}
