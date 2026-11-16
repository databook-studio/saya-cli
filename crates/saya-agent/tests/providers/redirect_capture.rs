/// Cross-host 307 capture rig shared by the three remote-provider redirect
/// tests. The origin answers 307 to `target_url`; the target records every
/// request that reaches it and answers `terminal` (a per-provider
/// terminal payload) so a following client terminates instead of erroring.
struct RedirectCapture {
    origin_base: String,
    reached: Arc<Mutex<Vec<String>>>,
    origin_handle: thread::JoinHandle<()>,
    target_handle: thread::JoinHandle<()>,
}
