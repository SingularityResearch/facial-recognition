//! Where the neural networks run. One model runtime on every platform (ONNX Runtime), on the processor or on
//! the platform's faster path. `FACE_DEVICE` = auto (default) | gpu | cpu.
//!
//! In auto, a model goes to a faster path only when that path is known or shown to be right:
//! - Windows: DirectML, measured (the same answers, 3 to 4 times the speed);
//! - Apple systems: CoreML as an ML program, after a check when the model is loaded: the same answers as the
//!   processor's (within `GPU_TOLERANCE`) and clearly faster (`NEEDS_GAIN`). CoreML's default mode failed
//!   that for a recognition model on a Mac and an iPhone alike (prints 0.2 to 0.3% off): the ML program mode
//!   passed;
//! - elsewhere the processor. On an Android phone neither XNNPACK nor NNAPI was faster (measured 2026-11-20).
//!
//! A check costs seconds at every start (each model run a dozen times on the processor and on the path
//! tried), so what it found is kept between starts where the program names a folder for it (`keeping`):
//! a model whose check chose the processor is loaded there at once, and one whose check chose a faster path
//! is loaded on it and run once against the processor's kept answer, which it must still give. A kept
//! decision holds for the same model file, the same program and system (`tag`) and the same rules; anything
//! else, and the full check runs again.
//!
//! What was learned the hard way is kept:
//! - one run at a time on a graphics processor in this process (two threads in DirectML at once crashed a
//!   graphics driver; loading a model while another ran crashed the runtime): `ACCEL`;
//! - a graphics session that failed is never destroyed (`LOST`): tearing one down after its device was lost
//!   hung two apps for good. The processor takes over for the rest of the process.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime},
};

use ort::{environment::Environment, session::Session, value::TensorRef};

use crate::{Error, Result};

/// The largest relative difference from the processor's answer (|gpu - cpu| / |cpu|) that counts as the same.
/// A measured value.
pub const GPU_TOLERANCE: f64 = 0.01;
/// A faster path must take at most this share of the processor's time: a gain inside the noise of two
/// timings would be chosen at one start and not at the next.
pub const NEEDS_GAIN: f64 = 0.9;

/// A kept decision for the processor is checked again after this many seconds (a day). A check's timings
/// are those of its moment: on a phone that had run warm for hours, the faster path for face finding came
/// out slower than the processor (82.9 ms against 71.5, where it had been 12.1 against 19.4 that morning),
/// and that must not stand for long. A system's update can change it too. Checking the models the
/// processor was chosen for again costs a start a few seconds once a day; a decision for a faster path
/// needs no such date, since it is tried against the processor's answer at every start.
const CHECK_AGAIN: u64 = 24 * 3600;
/// What a kept decision was made under: changed whenever the check's rules change.
const RULES: &str = "1";

/// Models may be loaded side by side (`Faces::load` does), but a full check times its runs, and those
/// timings are only worth something taken alone: a full check holds this for writing, every other load for
/// reading.
static CHECKING: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// One model run or load at a time on the graphics processor, process-wide.
static ACCEL: Mutex<()> = Mutex::new(());
/// Graphics sessions that failed: kept for as long as the process lives.
static LOST: Mutex<Vec<Session>> = Mutex::new(Vec::new());
static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();
/// What each model's check found, for the log and for Settings: (model file, what was decided).
static DECISIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// What the checks at load decided, model by model.
pub fn decisions() -> Vec<(String, String)> {
    DECISIONS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn decided(path: &Path, text: String) {
    let file = file_name(path);
    eprintln!("{file}: {text}");
    let mut d = DECISIONS.lock().unwrap_or_else(|e| e.into_inner());
    d.retain(|(f, t)| !(f == &file && t == &text));
    d.push((file, text));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Gpu,
    Cpu,
}

impl Choice {
    pub fn from_env() -> Choice {
        match std::env::var("FACE_DEVICE").unwrap_or_default().to_lowercase().as_str() {
            "cpu" => Choice::Cpu,
            "gpu" => Choice::Gpu,
            _ => Choice::Auto,
        }
    }
}

/// A faster path than the plain processor one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    DirectMl,
    /// the processor through XNNPACK's kernels (phones, ARM boards)
    Xnnpack,
    /// Android's own path to a phone's graphics or neural processor
    Nnapi,
    /// Apple's, as the runtime sets it up by itself (a neural network on every unit, half precision on the
    /// Neural Engine), or as `FACE_COREML_FORMAT` and `FACE_COREML_UNITS` say: for measuring
    CoreMl,
    /// Apple's, as an ML program: full precision, the one of CoreML's ways that gives the processor's prints
    CoreMlProgram,
}

impl Gpu {
    pub fn name(self) -> &'static str {
        match self {
            Gpu::DirectMl => "DirectML",
            Gpu::Xnnpack => "XNNPACK",
            Gpu::Nnapi => "NNAPI",
            Gpu::CoreMl => "CoreML",
            Gpu::CoreMlProgram => "CoreML, ML program",
        }
    }

    fn label(self) -> String {
        match self {
            Gpu::Xnnpack => "CPU (XNNPACK)".to_string(),
            _ => format!("GPU ({})", self.name()),
        }
    }

    /// `FACE_ACCEL`: one path, used without a check. For measuring a path on a device.
    fn asked() -> Option<Gpu> {
        match std::env::var("FACE_ACCEL").unwrap_or_default().to_lowercase().as_str() {
            "directml" => Some(Gpu::DirectMl),
            "xnnpack" => Some(Gpu::Xnnpack),
            "nnapi" => Some(Gpu::Nnapi),
            "coreml" => Some(Gpu::CoreMl),
            "coreml-program" => Some(Gpu::CoreMlProgram),
            _ => None,
        }
    }
}

/// How this process decides where a model runs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Plan {
    Cpu,
    /// a path used without a check: measured for the whole system (DirectML), or asked for
    Take(Gpu, &'static str),
    /// paths tried at load against the processor: the same answers, and clearly faster
    Check(Vec<Gpu>),
}

/// A session for a model on the processor, or on a faster path (an error when that path doesn't take it).
///
/// `cache`: a folder where CoreML may keep the model as it compiled it, so that it isn't compiled at every
/// start (seconds for the recognition model). The folder is the model file's own (`Keep::compiled`).
fn session(env: &Environment, path: &Path, gpu: Option<Gpu>, cache: Option<&Path>) -> Result<Session> {
    let mut b = Session::builder(env)?;
    match gpu {
        #[cfg(not(target_os = "ios"))]
        Some(Gpu::DirectMl) => {
            // DirectML takes neither memory patterns nor parallel execution
            b = b.with_memory_pattern(false)?.with_parallel_execution(false)?;
            b = b.with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])?;
        }
        Some(Gpu::Xnnpack) => {
            b = b.with_execution_providers([ort::ep::XNNPACK::default().build().error_on_failure()])?;
        }
        #[cfg(not(target_os = "ios"))]
        Some(Gpu::Nnapi) => {
            b = b.with_execution_providers([ort::ep::NNAPI::default().build().error_on_failure()])?;
        }
        Some(Gpu::CoreMl) => {
            use ort::ep::coreml::{ComputeUnits, ModelFormat};
            let var = |k: &str| std::env::var(k).unwrap_or_default().to_lowercase();
            let mut ep = ort::ep::CoreML::default();
            match var("FACE_COREML_FORMAT").as_str() {
                "mlprogram" => ep = ep.with_model_format(ModelFormat::MLProgram),
                "neuralnetwork" => ep = ep.with_model_format(ModelFormat::NeuralNetwork),
                _ => {}
            }
            match var("FACE_COREML_UNITS").as_str() {
                "all" => ep = ep.with_compute_units(ComputeUnits::All),
                "gpu" => ep = ep.with_compute_units(ComputeUnits::CPUAndGPU),
                "ane" => ep = ep.with_compute_units(ComputeUnits::CPUAndNeuralEngine),
                "cpu" => ep = ep.with_compute_units(ComputeUnits::CPUOnly),
                _ => {}
            }
            b = b.with_execution_providers([ep.build().error_on_failure()])?;
        }
        Some(Gpu::CoreMlProgram) => {
            use ort::ep::coreml::{ComputeUnits, ModelFormat};
            let mut ep = ort::ep::CoreML::default()
                .with_model_format(ModelFormat::MLProgram)
                .with_compute_units(ComputeUnits::All);
            if let Some(dir) = cache {
                ep = ep.with_model_cache_dir(dir.to_string_lossy());
            }
            b = b.with_execution_providers([ep.build().error_on_failure()])?;
        }
        // linked into an iOS app, the runtime has only Apple's paths
        #[cfg(target_os = "ios")]
        Some(Gpu::DirectMl | Gpu::Nnapi) => return Err(Error::msg("this system has no such path")),
        None => {}
    }
    Ok(b.commit_from_file(path)?)
}

/// The file name the runtime's library has on this system.
pub fn library_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_vendor = "apple") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// The model runtime's own switch that keeps its telemetry from ever being set up.
pub const NO_TELEMETRY: &str = "ORT_DISABLE_TELEMETRY";

/// Switch the model runtime's telemetry off for this process, before the runtime is loaded. Every program
/// calls this first thing, while only its first thread runs.
///
/// ONNX Runtime from 1.30 on carries Microsoft's telemetry client on every system but Windows (a Mac's,
/// Android's and an iPhone's 1.31.1 have it; the others' 1.24.4 has none), and it is on unless this variable
/// says otherwise: at its start the runtime then makes a device id and a store of its own in the user's
/// folders and sends Microsoft what it runs on (the program's name, the system, the processor, that id).
/// With the variable set to 1 before
/// the runtime's environment is made, nothing of it is set up: no file, no id, no connection (tried on a Mac
/// with `examples/quiet.rs`). Nothing this app carries may send anything anywhere by itself: before a runtime's
/// pin is changed, that example is run with the new library, and the library is looked through for addresses.
pub fn no_telemetry() {
    if std::env::var_os(NO_TELEMETRY).is_none_or(|set| set != "1") {
        // SAFETY: called at a program's start, before it starts a thread; and where the runtime is loaded
        // (`env`) only when a program forgot to, once, before anything of the runtime reads it
        unsafe { std::env::set_var(NO_TELEMETRY, "1") };
    }
}

/// Where the runtime's library is: `FACE_ORT_LIB`, else beside the program, else in `<root>/runtime`.
pub fn find_library(root: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "ios") {
        return Some(PathBuf::from("linked into the app")); // nothing to find: see Cargo.toml
    }
    if let Some(p) = std::env::var_os("FACE_ORT_LIB").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(library_name())));
    [beside, Some(root.join("runtime").join(library_name()))].into_iter().flatten().find(|p| p.is_file())
}

fn env(library: &Path) -> Result<&'static Environment> {
    ENV.get_or_init(|| {
        // One pool of worker threads for every model. Left to itself each model gets a pool of its own, and
        // a pool keeps its threads spinning for a while after a run: the next model's run then fought the
        // last one's idle threads for the processor's cores (measured on the processor alone: 37 to 47 ms a
        // picture with a pool each, against 24 with the one pool).
        //
        // The library is loaded by its full path first, and nothing of the runtime is touched before that:
        // asked for by name alone, Windows hands out its own older copy from System32 (1.17.1 there), which
        // this can't use.
        //
        // And before anything of it runs, its telemetry is switched off (`no_telemetry`): the programs do that
        // at their start; here it is made sure of.
        no_telemetry();
        #[cfg(not(target_os = "ios"))]
        let builder = ort::init_from(library)
            .map_err(|e| format!("The model runtime couldn't be loaded from {}: {e}", library.display()))?;
        #[cfg(target_os = "ios")]
        let builder = {
            let _ = library;
            ort::init()
        };
        let mut pool = ort::environment::GlobalThreadPoolOptions::default();
        // FACE_THREADS: how many threads a model's run uses (for measuring: a phone's small cores can slow a
        // run that is spread over all of them). Left out, the runtime takes the processor's cores.
        if let Some(n) = std::env::var("FACE_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0) {
            pool = pool.with_intra_threads(n).map_err(|e| format!("FACE_THREADS: {e}"))?;
        }
        builder
            .with_name("face")
            // (the crate's own default is to switch the runtime's telemetry ON: never leave this line out)
            .with_telemetry(false)
            .with_global_thread_pool(pool)
            .build()
            .map_err(|e| format!("The model runtime didn't start: {e}"))
    })
    .as_ref()
    .map_err(|e| Error::msg(e.clone()))
}

/// The made-up input a model is tried with at its check: its shape and the range of its values.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub shape: [usize; 4],
    pub low: f32,
    pub high: f32,
}

impl Probe {
    /// an aligned face as the recognition, quality and expression models take it
    pub fn face() -> Probe {
        Probe { shape: [1, 3, 112, 112], low: -1.0, high: 1.0 }
    }
    /// a picture as the face detector takes it (a photo's pixel range)
    pub fn frame() -> Probe {
        Probe { shape: [1, 3, 640, 640], low: 0.0, high: 255.0 }
    }

    /// The same values at every start: each from a hash of its place.
    fn input(&self) -> Vec<f32> {
        let n: usize = self.shape.iter().product();
        (0..n as u32)
            .map(|i| {
                let mut v = i.wrapping_mul(0x9E37_79B1);
                v ^= v >> 15;
                v = v.wrapping_mul(0x85EB_CA77);
                v ^= v >> 13;
                self.low + (v as f64 / 4_294_967_296.0) as f32 * (self.high - self.low)
            })
            .collect()
    }
}

/// One faster path's showing at a model's check: how far its answer was from the processor's and how long a
/// run took, or why it gave none.
#[derive(Debug, Clone)]
struct Trial {
    name: &'static str,
    outcome: std::result::Result<(f64, f64), String>,
}

/// Which of the tried paths a model runs on (None: the processor), and a line for each about why. A path is
/// taken only with the processor's answers and clearly faster than it; of several, the fastest.
fn judge(cpu_ms: f64, trials: &[Trial]) -> (Option<usize>, Vec<String>) {
    let good = |t: &Trial| matches!(t.outcome, Ok((diff, ms)) if diff <= GPU_TOLERANCE && ms < cpu_ms * NEEDS_GAIN);
    let ms_of = |t: &Trial| t.outcome.as_ref().map_or(f64::MAX, |o| o.1);
    let best = (0..trials.len())
        .filter(|&i| good(&trials[i]))
        .min_by(|&a, &b| ms_of(&trials[a]).total_cmp(&ms_of(&trials[b])));
    let lines = trials
        .iter()
        .enumerate()
        .map(|(i, t)| match &t.outcome {
            Err(why) => format!("{}: {why}: CPU", t.name),
            Ok((diff, _)) if !diff.is_finite() || *diff > GPU_TOLERANCE => {
                format!("{}: different results from the CPU ({:.1}% apart): CPU", t.name, diff * 100.0)
            }
            Ok((diff, ms)) if Some(i) == best => {
                format!(
                    "{}: same results ({:.2}% apart), {ms:.1} ms vs {cpu_ms:.1} ms on the CPU: GPU",
                    t.name,
                    diff * 100.0
                )
            }
            Ok((_, ms)) if good(t) => format!("{}: same results, {ms:.1} ms, but another path is faster", t.name),
            Ok((_, ms)) => {
                format!(
                    "{}: same results but not clearly faster ({ms:.1} ms vs {cpu_ms:.1} ms on the CPU): CPU",
                    t.name
                )
            }
        })
        .collect();
    (best, lines)
}

/// A step's median time in milliseconds over `runs`, after `warm` runs (a graphics processor raises its
/// clock under load, and sets itself up at its first run). The first error ends it.
fn median_ms(runs: usize, warm: usize, mut step: impl FnMut() -> Result<()>) -> Result<f64> {
    for _ in 0..warm {
        step()?;
    }
    let mut ms = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        step()?;
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(|a, b| a.total_cmp(b));
    Ok(ms[ms.len() / 2])
}

/// How far apart two answers are: |a - b| / |b| over every output's values.
fn apart(a: &[Vec<f32>], b: &[Vec<f32>]) -> f64 {
    if a.len() != b.len() || a.iter().zip(b).any(|(x, y)| x.len() != y.len()) {
        return f64::INFINITY;
    }
    let (mut diff, mut norm) = (0f64, 0f64);
    for (x, y) in a.iter().flatten().zip(b.iter().flatten()) {
        if !x.is_finite() {
            return f64::INFINITY;
        }
        diff += (*x as f64 - *y as f64).powi(2);
        norm += (*y as f64).powi(2);
    }
    diff.sqrt() / norm.sqrt().max(1e-6)
}

/// What a model's check found, as kept between starts.
enum Kept {
    Cpu,
    /// the path taken, and the processor's answer to the probe, which the path must still give
    Gpu(Gpu, Vec<Vec<f32>>),
}

/// Where the checks' findings are kept, and what they hold for.
#[derive(Clone)]
struct Keep {
    dir: PathBuf,
    tag: String,
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Keep {
    fn note(&self, model: &Path) -> PathBuf {
        self.dir.join(format!("{}.check", file_name(model)))
    }

    fn answer(&self, model: &Path) -> PathBuf {
        self.dir.join(format!("{}.want", file_name(model)))
    }

    /// What a kept finding must have been made for: this program and system, this very model file, this
    /// probe, these paths, these rules.
    fn made_for(&self, model: &Path, probe: &Probe, candidates: &[Gpu]) -> String {
        let size = std::fs::metadata(model).map_or(0, |m| m.len());
        let names: Vec<&str> = candidates.iter().map(|g| g.name()).collect();
        format!("rules {RULES} | {} | {size} bytes | {:?} | {}", self.tag, probe.shape, names.join(" / "))
    }

    /// The folder for CoreML's compiled copy of this very model file (its size and time in the name: another
    /// file under the same name gets another folder, and this one's older folders are removed).
    fn compiled(&self, model: &Path) -> Option<PathBuf> {
        let meta = std::fs::metadata(model).ok()?;
        let time = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
        let (root, name) = (self.dir.join("compiled"), file_name(model));
        let dir = root.join(format!("{name}-{}-{time}", meta.len()));
        if !dir.is_dir() {
            for old in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                if old.file_name().to_string_lossy().starts_with(&format!("{name}-")) {
                    let _ = std::fs::remove_dir_all(old.path());
                }
            }
            std::fs::create_dir_all(&dir).ok()?;
        }
        Some(dir)
    }

    /// The kept finding for a model, when there is one and it was made for exactly this.
    fn read(&self, model: &Path, probe: &Probe, candidates: &[Gpu]) -> Option<Kept> {
        let text = std::fs::read_to_string(self.note(model)).ok()?;
        let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='));
        if value("for")? != self.made_for(model, probe, candidates) {
            return None;
        }
        let at: u64 = value("at")?.parse().ok()?;
        match value("path")? {
            "CPU" => (unix_now().saturating_sub(at) < CHECK_AGAIN).then_some(Kept::Cpu),
            name => {
                let gpu = *candidates.iter().find(|g| g.name() == name)?;
                let raw = std::fs::read(self.answer(model)).ok()?;
                let mut outputs = Vec::new();
                let mut rest = &raw[..];
                while !rest.is_empty() {
                    let n = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
                    let body = rest.get(4..4 + n * 4)?;
                    outputs.push(body.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
                    rest = &rest[4 + n * 4..];
                }
                (!outputs.is_empty()).then_some(Kept::Gpu(gpu, outputs))
            }
        }
    }

    /// Keep a check's finding: the path taken (None: the processor) and, for a path, the processor's answer
    /// to the probe. A finding that can't be written is simply made again at the next start.
    fn write(&self, model: &Path, probe: &Probe, candidates: &[Gpu], taken: Option<Gpu>, want: &[Vec<f32>]) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let _ = std::fs::remove_file(self.note(model)); // never a note with another finding's answer beside it
        if taken.is_some() {
            let mut raw = Vec::new();
            for output in want {
                raw.extend_from_slice(&(output.len() as u32).to_le_bytes());
                raw.extend(output.iter().flat_map(|v| v.to_le_bytes()));
            }
            if std::fs::write(self.answer(model), raw).is_err() {
                return;
            }
        }
        let text = format!(
            "for={}\npath={}\nat={}\n",
            self.made_for(model, probe, candidates),
            taken.map_or("CPU", |g| g.name()),
            unix_now()
        );
        let _ = std::fs::write(self.note(model), text);
    }
}

pub struct Runtime {
    env: &'static Environment,
    pub choice: Choice,
    plan: Plan,
    keep: Option<Keep>,
}

impl Runtime {
    pub fn new(library: &Path, choice: Choice) -> Result<Runtime> {
        let candidates: Vec<Gpu> = if cfg!(windows) {
            vec![Gpu::DirectMl]
        } else if cfg!(target_vendor = "apple") {
            vec![Gpu::CoreMlProgram]
        } else {
            Vec::new()
        };
        let plan = match (choice, Gpu::asked(), candidates.first()) {
            (Choice::Cpu, _, _) => Plan::Cpu,
            (_, Some(asked), _) => Plan::Take(asked, "asked for with FACE_ACCEL (not checked)"),
            (Choice::Gpu, None, Some(&first)) => Plan::Take(first, "asked for with FACE_DEVICE=gpu (not checked)"),
            (Choice::Auto, None, Some(&Gpu::DirectMl)) => Plan::Take(Gpu::DirectMl, ""),
            (Choice::Auto, None, Some(_)) => Plan::Check(candidates),
            (_, None, None) => Plan::Cpu,
        };
        Ok(Runtime { env: env(library)?, choice, plan, keep: None })
    }

    /// Keep what the checks at load find in `dir` between starts (and there CoreML's compiled models), for as
    /// long as `tag` is the same: it names what the findings depend on beyond the model file, which is
    /// this program's version and the system it runs on.
    pub fn keeping(mut self, dir: &Path, tag: &str) -> Runtime {
        self.keep = Some(Keep { dir: dir.to_path_buf(), tag: tag.replace(['\n', '\r'], " ") });
        self
    }

    /// A model on the processor, whatever else this system has: for one that was measured
    /// there (the voice model), or that takes inputs of every length.
    pub fn model_on_cpu(&self, path: &Path, what: &'static str) -> Result<Model> {
        if !path.is_file() {
            return Err(Error::msg(format!("Missing model file: {}", path.display())));
        }
        Ok(Model::new(self.env, path, what, session(self.env, path, None, None)?, None, None))
    }

    /// A model where it is right and fastest: on a faster path this system has when that path is known or
    /// shown (with `probe`, at this load) to give the processor's answers clearly faster; else on the processor.
    pub fn model(&self, path: &Path, what: &'static str, probe: &Probe) -> Result<Model> {
        let began = Instant::now();
        let model = self.model_where_it_belongs(path, what, probe)?;
        eprintln!("{}: loaded in {:.1} s, on the {}", file_name(path), began.elapsed().as_secs_f64(), model.device());
        Ok(model)
    }

    fn model_where_it_belongs(&self, path: &Path, what: &'static str, probe: &Probe) -> Result<Model> {
        if !path.is_file() {
            return Err(Error::msg(format!("Missing model file: {}", path.display())));
        }
        // side by side with other loads, until a full check has to time its runs alone
        let mut alone = None;
        let side_by_side = Some(CHECKING.read().unwrap_or_else(|e| e.into_inner()));
        let cache = self.keep.as_ref().and_then(|k| k.compiled(path));
        let on = |gpu: Gpu| {
            // One load at a time on a graphics processor: the rule DirectML taught (a model loaded while
            // another ran took the runtime down). CoreML's loads may go side by side: each is seconds of
            // Apple's own preparing of the model, and two of them one after the other were most of an
            // iPhone's start (7 to 8 s). Runs stay one at a time everywhere.
            let _one = (!matches!(gpu, Gpu::CoreMl | Gpu::CoreMlProgram))
                .then(|| ACCEL.lock().unwrap_or_else(|e| e.into_inner()));
            session(self.env, path, Some(gpu), cache.as_deref())
        };
        match &self.plan {
            Plan::Cpu => {}
            Plan::Take(gpu, why) => match on(*gpu) {
                Ok(s) => {
                    if !why.is_empty() {
                        decided(path, format!("{}, {why}", gpu.name()));
                    }
                    return Ok(Model::new(self.env, path, what, s, Some(*gpu), cache));
                }
                Err(e) => decided(path, format!("{} couldn't load it ({}): CPU", gpu.name(), clip(&e.0, 120))),
            },
            Plan::Check(candidates) => {
                let x = probe.input();
                // what this model's check found at an earlier start, when it was found for exactly this
                match self.keep.as_ref().and_then(|k| k.read(path, probe, candidates)) {
                    Some(Kept::Cpu) => {
                        decided(path, "CPU, as its check found at an earlier start".to_string());
                        return Ok(Model::new(self.env, path, what, session(self.env, path, None, None)?, None, None));
                    }
                    Some(Kept::Gpu(gpu, want)) => {
                        // loaded on the path its check chose and run once: it must still give the
                        // processor's answer, which was kept with the finding
                        if let Ok(mut s) = on(gpu) {
                            let input = session_input(&s);
                            let got = {
                                let _one = ACCEL.lock().unwrap_or_else(|e| e.into_inner());
                                run_once(&mut s, &input, &probe.shape, &x)
                            };
                            let diff = got.map_or(f64::INFINITY, |got| apart(&got, &want));
                            if diff.is_finite() && diff <= GPU_TOLERANCE {
                                let line = format!(
                                    "{}: the processor's results still ({:.2}% apart), as checked at an earlier start: GPU",
                                    gpu.name(),
                                    diff * 100.0
                                );
                                decided(path, line);
                                return Ok(Model::new(self.env, path, what, s, Some(gpu), cache));
                            }
                        }
                        decided(
                            path,
                            "what its check found at an earlier start no longer holds: checked again".to_string(),
                        );
                    }
                    None => {}
                }
                drop(side_by_side);
                alone.replace(CHECKING.write().unwrap_or_else(|e| e.into_inner()));
                let mut cpu = session(self.env, path, None, None)?;
                let input = session_input(&cpu);
                let want = run_once(&mut cpu, &input, &probe.shape, &x)?;
                let cpu_ms = median_ms(10, 3, || run_once(&mut cpu, &input, &probe.shape, &x).map(drop))?;
                let mut made: Vec<Option<Session>> = Vec::new();
                let mut trials = Vec::new();
                for &gpu in candidates {
                    let mut s = on(gpu);
                    let outcome = match &mut s {
                        Err(e) => Err(format!("couldn't load it ({})", clip(&e.0, 120))),
                        Ok(s) => {
                            let _one = ACCEL.lock().unwrap_or_else(|e| e.into_inner());
                            run_once(s, &input, &probe.shape, &x)
                                .and_then(|got| {
                                    let diff = apart(&got, &want);
                                    // a path with other answers isn't timed: it is out either way
                                    if diff.is_finite() && diff <= GPU_TOLERANCE {
                                        median_ms(10, 3, || run_once(s, &input, &probe.shape, &x).map(drop))
                                            .map(|ms| (diff, ms))
                                    } else {
                                        Ok((diff, f64::MAX))
                                    }
                                })
                                .map_err(|e| format!("test run failed ({})", clip(&e.0, 120)))
                        }
                    };
                    trials.push(Trial { name: gpu.name(), outcome });
                    made.push(s.ok());
                }
                let (best, lines) = judge(cpu_ms, &trials);
                for line in lines {
                    decided(path, line);
                }
                // kept for the next start, unless a path couldn't be tried at all: that may be different
                // then, and must not stand as "the processor" for a month
                if let (Some(k), true) = (&self.keep, trials.iter().all(|t| t.outcome.is_ok())) {
                    k.write(path, probe, candidates, best.map(|i| candidates[i]), &want);
                }
                if let Some(i) = best {
                    let s = made[i].take().expect("the chosen path's session");
                    return Ok(Model::new(self.env, path, what, s, Some(candidates[i]), cache));
                }
                drop(alone);
                return Ok(Model::new(self.env, path, what, cpu, None, None));
            }
        }
        Ok(Model::new(self.env, path, what, session(self.env, path, None, None)?, None, None))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn clip(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

fn session_input(session: &Session) -> String {
    session.inputs().first().map(|i| i.name().to_string()).unwrap_or_default()
}

struct State {
    /// a session per input shape on a graphics processor (it sets itself up again whenever the shape differs
    /// from the run before: about 30 ms measured); one session for every shape on the processor
    sessions: HashMap<Vec<usize>, Session>,
    first: Option<Vec<usize>>,
    gpu: Option<Gpu>,
}

/// One loaded model. `run` may be called from any thread.
pub struct Model {
    env: &'static Environment,
    path: PathBuf,
    what: &'static str,
    input: String,
    /// most input shapes that get a graphics session of their own (`DET_SESSIONS`); the first one's
    /// session takes the rest
    pub per_shape: usize,
    state: Mutex<State>,
    device: Mutex<String>,
    /// where CoreML keeps this model compiled, for the further sessions a graphics path gets
    cache: Option<PathBuf>,
}

impl Model {
    fn new(
        env: &'static Environment,
        path: &Path,
        what: &'static str,
        session: Session,
        gpu: Option<Gpu>,
        cache: Option<PathBuf>,
    ) -> Model {
        let input = session_input(&session);
        let device = gpu.map_or_else(|| "CPU".to_string(), Gpu::label);
        let mut sessions = HashMap::new();
        sessions.insert(Vec::new(), session);
        Model {
            env,
            path: path.to_path_buf(),
            what,
            input,
            per_shape: 1,
            state: Mutex::new(State { sessions, first: None, gpu }),
            device: Mutex::new(device),
            cache,
        }
    }

    /// Where it runs now, for the log and Settings.
    pub fn device(&self) -> String {
        self.device.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn on_gpu(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).gpu.is_some()
    }

    /// Every output of one run, each as its values in order.
    pub fn run(&self, shape: &[usize], data: &[f32]) -> Result<Vec<Vec<f32>>> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.gpu.is_some() {
            let _one = ACCEL.lock().unwrap_or_else(|e| e.into_inner());
            let key = self.session_for(&mut st, shape);
            match run_once(st.sessions.get_mut(&key).expect("a session for the shape"), &self.input, shape, data) {
                Ok(out) => return Ok(out),
                Err(e) => {
                    eprintln!("{} on the GPU failed ({}); using the CPU from now on", self.what, clip(&e.0, 160));
                    let cpu = session(self.env, &self.path, None, None)?;
                    let mut lost = LOST.lock().unwrap_or_else(|e| e.into_inner());
                    lost.extend(std::mem::take(&mut st.sessions).into_values()); // never destroyed: see LOST
                    st.sessions.insert(Vec::new(), cpu);
                    st.gpu = None;
                    *self.device.lock().unwrap_or_else(|e| e.into_inner()) = "CPU (after a GPU error)".to_string();
                }
            }
        }
        run_once(st.sessions.get_mut(&Vec::new()).expect("the session"), &self.input, shape, data)
    }

    /// The key of the graphics session that takes this shape, making one for a shape not seen before while
    /// there is room.
    fn session_for(&self, st: &mut State, shape: &[usize]) -> Vec<usize> {
        if self.per_shape <= 1 {
            return Vec::new();
        }
        if st.sessions.contains_key(shape) {
            return shape.to_vec();
        }
        if st.first.is_none() {
            // the session made at load takes the first shape seen
            let s = st.sessions.remove(&Vec::new()).expect("the first session");
            st.sessions.insert(shape.to_vec(), s);
            st.first = Some(shape.to_vec());
            return shape.to_vec();
        }
        if st.sessions.len() < self.per_shape {
            if let Ok(s) = session(self.env, &self.path, st.gpu, self.cache.as_deref()) {
                st.sessions.insert(shape.to_vec(), s);
                return shape.to_vec();
            }
        }
        st.first.clone().expect("the first shape")
    }
}

/// A tensor given to a model that has several inputs: its shape and its values in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Tensor {
    F32(Vec<usize>, Vec<f32>),
    I64(Vec<usize>, Vec<i64>),
    Bool(Vec<usize>, Vec<bool>),
}

/// An output of a run: its name, its shape and its values in order.
pub type Output = (String, Vec<usize>, Vec<f32>);

/// One part of a model made of several (the describe model's four): named inputs of several kinds, named
/// outputs, on the processor. `run` may be called from any thread, one run at a time.
pub struct Parts {
    session: Mutex<Session>,
    /// each input's name
    pub inputs: Vec<String>,
    /// each output's name, in the model's order
    pub outputs: Vec<String>,
}

impl Runtime {
    /// A model with several inputs, on the processor.
    pub fn parts_on_cpu(&self, path: &Path) -> Result<Parts> {
        if !path.is_file() {
            return Err(Error::msg(format!("Missing model file: {}", path.display())));
        }
        let session = session(self.env, path, None, None)?;
        let inputs = session.inputs().iter().map(|i| i.name().to_string()).collect();
        let outputs = session.outputs().iter().map(|o| o.name().to_string()).collect();
        Ok(Parts { session: Mutex::new(session), inputs, outputs })
    }
}

impl Parts {
    /// One run. The outputs whose names `want` passes, in the model's order: (name, shape, values), each of
    /// 32-bit floats. An input is used where it lies, not copied.
    pub fn run(&self, inputs: &[(&str, &Tensor)], want: &dyn Fn(&str) -> bool) -> Result<Vec<Output>> {
        let mut given: Vec<(std::borrow::Cow<'_, str>, ort::session::SessionInputValue<'_>)> =
            Vec::with_capacity(inputs.len());
        for (name, tensor) in inputs {
            let value: ort::session::SessionInputValue<'_> = match tensor {
                Tensor::F32(shape, data) => TensorRef::from_array_view((shape.as_slice(), data.as_slice()))?.into(),
                Tensor::I64(shape, data) => TensorRef::from_array_view((shape.as_slice(), data.as_slice()))?.into(),
                Tensor::Bool(shape, data) => TensorRef::from_array_view((shape.as_slice(), data.as_slice()))?.into(),
            };
            given.push(((*name).into(), value));
        }
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let outputs = session.run(given)?;
        let mut out = Vec::new();
        for (k, name) in self.outputs.iter().enumerate() {
            if want(name) {
                let (shape, data) = outputs[k].try_extract_tensor::<f32>()?;
                out.push((name.clone(), shape.iter().map(|d| *d as usize).collect(), data.to_vec()));
            }
        }
        Ok(out)
    }
}

fn run_once(session: &mut Session, input: &str, shape: &[usize], data: &[f32]) -> Result<Vec<Vec<f32>>> {
    let tensor = TensorRef::from_array_view((shape, data))?;
    let outputs = session.run(ort::inputs![input => tensor])?;
    let mut out = Vec::with_capacity(outputs.len());
    for i in 0..outputs.len() {
        out.push(outputs[i].try_extract_tensor::<f32>()?.1.to_vec());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(name: &'static str, outcome: std::result::Result<(f64, f64), &str>) -> Trial {
        Trial { name, outcome: outcome.map_err(str::to_string) }
    }

    #[test]
    fn a_checks_finding_is_kept_for_exactly_what_it_was_made_for() {
        let dir = std::env::temp_dir().join(format!("face-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("some_model.onnx");
        std::fs::write(&model, b"a model's bytes").unwrap();
        let keep = Keep { dir: dir.join("checks"), tag: "program 1 | system 1".to_string() };
        let (probe, paths) = (Probe::face(), [Gpu::CoreMlProgram]);
        assert!(keep.read(&model, &probe, &paths).is_none(), "nothing kept yet");

        // a path was taken: kept with the processor's answer, which comes back value for value
        let want = vec![vec![0.5f32, -1.25, 3.0e-7], vec![], vec![42.0]];
        keep.write(&model, &probe, &paths, Some(Gpu::CoreMlProgram), &want);
        match keep.read(&model, &probe, &paths) {
            Some(Kept::Gpu(Gpu::CoreMlProgram, kept)) => assert_eq!(kept, want),
            _ => panic!("the finding wasn't kept"),
        }
        // not for another program or system, another probe, other paths, or another file under the name
        let other = Keep { dir: keep.dir.clone(), tag: "program 2 | system 1".to_string() };
        assert!(other.read(&model, &probe, &paths).is_none());
        assert!(keep.read(&model, &Probe::frame(), &paths).is_none());
        assert!(keep.read(&model, &probe, &[Gpu::CoreMl, Gpu::CoreMlProgram]).is_none());
        std::fs::write(&model, b"another model's bytes, longer").unwrap();
        assert!(keep.read(&model, &probe, &paths).is_none());

        // the processor was chosen: kept without an answer, and only for a time
        keep.write(&model, &probe, &paths, None, &want);
        assert!(matches!(keep.read(&model, &probe, &paths), Some(Kept::Cpu)));
        assert!(!keep.answer(&model).exists() || keep.read(&model, &probe, &paths).is_some());
        let note = std::fs::read_to_string(keep.note(&model)).unwrap();
        let old = note.replace(
            &format!("at={}", note.split("at=").nth(1).unwrap().trim()),
            &format!("at={}", unix_now() - CHECK_AGAIN - 5),
        );
        std::fs::write(keep.note(&model), old).unwrap();
        assert!(keep.read(&model, &probe, &paths).is_none(), "checked again after a month");

        // a path's finding whose answer is gone or cut short is no finding
        keep.write(&model, &probe, &paths, Some(Gpu::CoreMlProgram), &want);
        std::fs::write(keep.answer(&model), [3u8, 0, 0, 0, 1, 2]).unwrap();
        assert!(keep.read(&model, &probe, &paths).is_none());
        std::fs::remove_file(keep.answer(&model)).unwrap();
        assert!(keep.read(&model, &probe, &paths).is_none());

        // CoreML's compiled copy has a folder of the model file's own; another file under the name gets another
        let first = keep.compiled(&model).unwrap();
        assert!(first.is_dir() && keep.compiled(&model).unwrap() == first);
        std::fs::write(&model, b"a third model").unwrap();
        let second = keep.compiled(&model).unwrap();
        assert!(second != first && second.is_dir() && !first.exists(), "the older folder is removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_faster_path_needs_the_same_answers_and_a_clear_gain() {
        // the same answers, five times the speed: taken
        let (best, lines) = judge(16.0, &[trial("A", Ok((0.0012, 3.1)))]);
        assert_eq!(best, Some(0));
        assert_eq!(lines, ["A: same results (0.12% apart), 3.1 ms vs 16.0 ms on the CPU: GPU"]);
        // other answers, however fast: never
        let (best, lines) = judge(16.0, &[trial("A", Ok((0.057, 1.4)))]);
        assert_eq!(best, None);
        assert_eq!(lines, ["A: different results from the CPU (5.7% apart): CPU"]);
        assert_eq!(judge(16.0, &[trial("A", Ok((f64::INFINITY, 1.0)))]).0, None);
        assert_eq!(judge(16.0, &[trial("A", Ok((f64::NAN, 1.0)))]).0, None);
        // the same answers but slower, or faster only within the noise: the processor
        let (best, lines) = judge(4.6, &[trial("A", Ok((0.0, 8.8)))]);
        assert_eq!(best, None);
        assert_eq!(lines, ["A: same results but not clearly faster (8.8 ms vs 4.6 ms on the CPU): CPU"]);
        assert_eq!(judge(14.7, &[trial("A", Ok((0.0, 13.7)))]).0, None);
        // a path that didn't load or run
        let (best, lines) = judge(16.0, &[trial("A", Err("couldn't load it (no such provider)"))]);
        assert_eq!(best, None);
        assert_eq!(lines, ["A: couldn't load it (no such provider): CPU"]);
    }

    #[test]
    fn the_runtimes_telemetry_is_switched_off_before_it_is_loaded() {
        no_telemetry();
        assert_eq!(std::env::var(NO_TELEMETRY).as_deref(), Ok("1"));
        // whatever the environment said before: the app sends nothing, and nobody switches that on from outside
        // SAFETY: a test of this one variable; nothing else in these tests reads or writes it
        unsafe { std::env::set_var(NO_TELEMETRY, "0") };
        no_telemetry();
        assert_eq!(std::env::var(NO_TELEMETRY).as_deref(), Ok("1"));
        // and where the runtime is loaded: the switch first, and the crate told not to switch it on (its
        // own default does)
        // (as it is checked out: with either kind of line ending)
        let source = include_str!("runtime.rs").replace("\r\n", "\n");
        let loads = source.split("fn env(library: &Path)").nth(1).expect("the function that loads the runtime");
        let (switch, load) = (loads.find("no_telemetry();"), loads.find("ort::init_from(library)"));
        assert!(switch.is_some() && switch < load, "the switch comes before the library is loaded");
        let end = loads.find("\n}\n").expect("the function's end");
        assert!(loads[..end].contains(".with_telemetry(false)"));
    }

    #[test]
    fn of_several_right_paths_the_fastest() {
        let trials = [trial("A", Ok((0.001, 4.2))), trial("B", Ok((0.03, 1.4))), trial("C", Ok((0.002, 3.0)))];
        let (best, lines) = judge(16.0, &trials);
        assert_eq!(best, Some(2));
        assert!(lines[0].ends_with("but another path is faster"), "{}", lines[0]);
        assert!(lines[1].ends_with(": CPU"));
        assert!(lines[2].ends_with(": GPU"));
    }

    #[test]
    fn answers_are_compared_over_every_value() {
        let a = vec![vec![1.0f32, 2.0], vec![3.0]];
        assert_eq!(apart(&a, &a), 0.0);
        assert!((apart(&[vec![1.01f32]], &[vec![1.0f32]]) - 0.01).abs() < 1e-6);
        assert!(apart(&[vec![1.0f32]], &[vec![1.0f32, 2.0]]).is_infinite());
        assert!(apart(&[vec![f32::NAN]], &[vec![1.0f32]]).is_infinite());
    }

    #[test]
    fn the_made_up_input_is_the_same_every_time_and_in_range() {
        let p = Probe::frame();
        let x = p.input();
        assert_eq!(x.len(), 3 * 640 * 640);
        assert_eq!(x, p.input());
        assert!(x.iter().all(|v| (0.0..=255.0).contains(v)));
        assert!(x.iter().any(|v| *v > 200.0) && x.iter().any(|v| *v < 50.0));
    }
}
