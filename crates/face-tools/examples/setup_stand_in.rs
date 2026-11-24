//! A stand-in for an installer that Setup runs (Microsoft's WebView2 bootstrapper), for the installer's check
//! (a trial install that is taken away again): it installs nothing and only notes how it was started, one word a
//! line, in the file `FACE_TEST_SETUP_NOTE` names. The real bootstrapper is never run by a check: it would
//! download and install Microsoft's runtime on the computer the check runs on.

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if let Some(note) = std::env::var_os("FACE_TEST_SETUP_NOTE") {
        let _ = std::fs::write(note, words.join("\n"));
    }
}
