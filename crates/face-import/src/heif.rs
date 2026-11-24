//! HEIC photos (an iPhone's): decoded by libheif with libde265, always asked the same
//! way: the main picture, as shown (turned where the file says to turn it),
//! as 8-bit red, green, blue.
//!
//! The two are libraries of their own beside the program, loaded when the first HEIC photo is met, not built
//! into it: their licence (LGPL) asks that whoever has the program can put another build of them in their
//! place. They are looked for where `FACE_HEIF_LIB` names libheif's file, else beside the program, else in
//! `<root>/runtime`; libde265 beside libheif. Without them a HEIC photo can't be read, and the reason says so.

use std::{
    ffi::{CStr, c_char, c_int, c_void},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use libloading::Library;

use crate::decode::Rgb;

#[repr(C)]
struct HeifError {
    code: c_int,
    subcode: c_int,
    message: *const c_char,
}

/// The beginning of libheif's decoding options, as far as the one setting that is changed.
#[repr(C)]
struct Options {
    version: u8,
    ignore_transformations: u8,
    start_progress: *const c_void,
    on_progress: *const c_void,
    end_progress: *const c_void,
    progress_user_data: *mut c_void,
    convert_hdr_to_8bit: u8,
}

const COLORSPACE_RGB: c_int = 1;
const COLORSPACE_MONOCHROME: c_int = 2;
const CHROMA_MONOCHROME: c_int = 0;
const CHROMA_RGB: c_int = 10;
const CHROMA_RGBA: c_int = 11;
const CHANNEL_Y: c_int = 0;
const CHANNEL_INTERLEAVED: c_int = 10;
/// (it changes how fast a picture is decoded, not the picture)
const THREADS: c_int = 4;

struct Api {
    context_alloc: unsafe extern "C" fn() -> *mut c_void,
    context_free: unsafe extern "C" fn(*mut c_void),
    read_from_memory: unsafe extern "C" fn(*mut c_void, *const c_void, usize, *const c_void) -> HeifError,
    set_threads: Option<unsafe extern "C" fn(*mut c_void, c_int)>,
    primary_handle: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> HeifError,
    handle_release: unsafe extern "C" fn(*const c_void),
    has_alpha: unsafe extern "C" fn(*const c_void) -> c_int,
    preferred: unsafe extern "C" fn(*const c_void, *mut c_int, *mut c_int) -> HeifError,
    options_alloc: unsafe extern "C" fn() -> *mut Options,
    options_free: unsafe extern "C" fn(*mut Options),
    decode_image: unsafe extern "C" fn(*const c_void, *mut *mut c_void, c_int, c_int, *const Options) -> HeifError,
    plane: unsafe extern "C" fn(*const c_void, c_int, *mut c_int) -> *const u8,
    width: unsafe extern "C" fn(*const c_void) -> c_int,
    height: unsafe extern "C" fn(*const c_void) -> c_int,
    image_release: unsafe extern "C" fn(*const c_void),
    version: unsafe extern "C" fn() -> *const c_char,
    /// kept loaded for as long as the program runs
    _libraries: Vec<Library>,
}

// SAFETY: the functions are libheif's own, which may be called from any thread (each call here has a
// context of its own), and the libraries are never unloaded
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn names() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("heif.dll", "libde265.dll")
    } else if cfg!(target_os = "macos") {
        ("libheif.1.dylib", "libde265.0.dylib")
    } else {
        ("libheif.so.1", "libde265.so.0")
    }
}

/// Where libheif's library is, if it is anywhere.
pub fn find_library() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FACE_HEIF_LIB").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let name = names().0;
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name)));
    [beside, Some(face_files::paths::root().join("runtime").join(name))].into_iter().flatten().find(|p| p.is_file())
}

fn load(file: &Path) -> Result<Api, String> {
    let said = |e: libloading::Error| format!("{}: {e}", file.display());
    let mut libraries = Vec::new();
    // the HEVC decoder first, by its full path: libheif asks for it by name, and finds the one that is loaded
    if let Some(de265) = file.parent().map(|d| d.join(names().1)).filter(|p| p.is_file()) {
        // SAFETY: loading a library runs its start-up code; these are the decoders this program ships
        libraries.push(unsafe { Library::new(&de265) }.map_err(said)?);
    }
    let heif = unsafe { Library::new(file) }.map_err(said)?;
    macro_rules! get {
        ($name:literal) => {
            // SAFETY: the name and the signature are libheif's own (heif.h)
            *unsafe { heif.get(concat!($name, "\0").as_bytes()) }.map_err(said)?
        };
    }
    let api = Api {
        context_alloc: get!("heif_context_alloc"),
        context_free: get!("heif_context_free"),
        read_from_memory: get!("heif_context_read_from_memory_without_copy"),
        set_threads: unsafe { heif.get(b"heif_context_set_max_decoding_threads\0") }.ok().map(|f| *f),
        primary_handle: get!("heif_context_get_primary_image_handle"),
        handle_release: get!("heif_image_handle_release"),
        has_alpha: get!("heif_image_handle_has_alpha_channel"),
        preferred: get!("heif_image_handle_get_preferred_decoding_colorspace"),
        options_alloc: get!("heif_decoding_options_alloc"),
        options_free: get!("heif_decoding_options_free"),
        decode_image: get!("heif_decode_image"),
        plane: get!("heif_image_get_plane_readonly"),
        width: get!("heif_image_get_primary_width"),
        height: get!("heif_image_get_primary_height"),
        image_release: get!("heif_image_release"),
        version: get!("heif_get_version"),
        _libraries: Vec::new(),
    };
    libraries.push(heif);
    Ok(Api { _libraries: libraries, ..api })
}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| match find_library() {
        Some(file) => load(&file).map_err(|e| format!("The HEIC decoder couldn't be loaded ({e}).")),
        None => Err(format!(
            "HEIC photos need libheif ({}), which wasn't found beside the program or in its runtime folder.",
            names().0
        )),
    })
    .as_ref()
    .map_err(String::clone)
}

/// libheif's version, where it is there: for the line that says what the photos are read with.
pub fn version() -> Option<String> {
    let api = api().ok()?;
    // SAFETY: a pointer to the library's own constant text
    Some(unsafe { CStr::from_ptr((api.version)()) }.to_string_lossy().into_owned())
}

/// Lets go of what libheif handed out when it goes out of scope.
struct Held(*const c_void, unsafe extern "C" fn(*const c_void));

impl Drop for Held {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handed out by libheif, released once
            unsafe { (self.1)(self.0) }
        }
    }
}

/// A file being read by libheif, let go of when it goes out of scope (after everything read from it).
struct Context(*mut c_void, unsafe extern "C" fn(*mut c_void));

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: handed out by libheif, freed once
        unsafe { (self.1)(self.0) }
    }
}

fn checked(e: HeifError) -> Result<(), String> {
    if e.code == 0 {
        return Ok(());
    }
    // SAFETY: libheif's message is a constant text, or nothing
    Err(if e.message.is_null() {
        "libheif couldn't read it".to_string()
    } else {
        unsafe { CStr::from_ptr(e.message) }.to_string_lossy().into_owned()
    })
}

/// The main picture of a HEIC file, as shown.
pub fn decode(data: &[u8]) -> Result<Rgb, String> {
    let api = api()?;
    // SAFETY: every pointer handed to libheif is one it handed out or one into `data`, which outlives the
    // context (it reads from it without a copy); what it hands out is released by `Held`
    unsafe {
        let context = (api.context_alloc)();
        if context.is_null() {
            return Err("libheif couldn't begin".to_string());
        }
        let context = Context(context, api.context_free);
        checked((api.read_from_memory)(context.0, data.as_ptr().cast(), data.len(), std::ptr::null()))?;
        if let Some(set_threads) = api.set_threads {
            set_threads(context.0, THREADS);
        }
        let mut handle = std::ptr::null_mut();
        checked((api.primary_handle)(context.0, &mut handle))?;
        let handle = Held(handle, api.handle_release);
        let alpha = (api.has_alpha)(handle.0) != 0;
        let (mut colorspace, mut chroma) = (COLORSPACE_RGB, CHROMA_RGB);
        let grey = (api.preferred)(handle.0, &mut colorspace, &mut chroma).code == 0
            && (colorspace, chroma) == (COLORSPACE_MONOCHROME, CHROMA_MONOCHROME)
            && !alpha;
        let (want_space, want_chroma, channel, bytes) = match (grey, alpha) {
            (true, _) => (COLORSPACE_MONOCHROME, CHROMA_MONOCHROME, CHANNEL_Y, 1),
            (false, true) => (COLORSPACE_RGB, CHROMA_RGBA, CHANNEL_INTERLEAVED, 4),
            (false, false) => (COLORSPACE_RGB, CHROMA_RGB, CHANNEL_INTERLEAVED, 3),
        };
        let options = (api.options_alloc)();
        if !options.is_null() {
            (*options).convert_hdr_to_8bit = 1;
        }
        let mut image = std::ptr::null_mut();
        let decoded = (api.decode_image)(handle.0, &mut image, want_space, want_chroma, options);
        if !options.is_null() {
            (api.options_free)(options);
        }
        checked(decoded)?;
        let image = Held(image, api.image_release);
        let mut stride: c_int = 0;
        let plane = (api.plane)(image.0, channel, &mut stride);
        let (w, h) = ((api.width)(image.0), (api.height)(image.0));
        if plane.is_null() || w <= 0 || h <= 0 || (stride as i64) < w as i64 * bytes as i64 {
            return Err("libheif gave no picture".to_string());
        }
        let (w, h, stride) = (w as usize, h as usize, stride as usize);
        let mut px = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            let row = std::slice::from_raw_parts(plane.add(y * stride), w * bytes);
            match bytes {
                1 => px.extend(row.iter().flat_map(|v| [*v; 3])),
                3 => px.extend_from_slice(row),
                _ => px.extend(row.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]])),
            }
        }
        Ok(Rgb { w, h, px })
    }
}
