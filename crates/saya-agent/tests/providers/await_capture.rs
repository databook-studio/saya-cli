fn await_capture(capture: RedirectCapture) -> (String, Arc<Mutex<Vec<String>>>) {
    capture.origin_handle.join().unwrap();
    capture.target_handle.join().unwrap();
    (capture.origin_base, capture.reached)
}
