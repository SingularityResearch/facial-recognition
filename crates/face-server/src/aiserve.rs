//! The describer's model and settings, suggested for the hardware of the computer its server runs on, and loaded
//! there (the owner, 2026-11-25: "suggest the best settings based on hardware", and "suggest and load the best
//! model with the best settings for the users hardware - decided when the settings are interacted with").
//!
//! - What a computer can hold (`Room`): its graphics processor's own memory, and for one built into the
//!   processor (or a Mac's unified memory) the computer's memory it may share; minus what the app's own models
//!   and the system keep (`KEEP_SEPARATE`, `KEEP_SHARED`).
//! - The models (`CANDIDATES`), best first for describing photos as measured on the owner's computers with the
//!   app's own request (the guide's "Describe photo's models" has the numbers), each with its name on every kind
//!   of server and its size at 4 bits with its picture part.
//! - `plan`: the best model the server has that fits wholly in the graphics memory (it runs at the graphics
//!   processor's speed), else the best that fits partly (a mixture of experts uses few of its weights a word, so
//!   it runs well with the rest in the computer's memory), with the settings for its kind: the context the task
//!   needs, every layer that fits on the graphics processor, one request at a time (three at once made a server
//!   forty times slower a request). Where a better model would fit than the server has, it says so.
//! - `load`: LM Studio on this computer is told to load it so (`lms load`), Ollama anywhere (a request that loads
//!   it with the context), and llama.cpp and vLLM, which serve what they were started with, get the command to
//!   start them with. It runs when the owner chooses the kind of server or the model in Settings, or presses
//!   Load: never by itself, and never a download.

use std::{path::Path, time::Duration};

use face_home::describe::{Config, where_is};
use serde_json::{Value, json};

use crate::hardware::GpuMemory;

/// GB a separate card keeps for the app's own models and the screen
const KEEP_SEPARATE: f64 = 1.5;
/// GB a built-in graphics processor (or a Mac) leaves to the system and the programs
const KEEP_SHARED: f64 = 8.0;
/// the context describing a photo needs: the picture with the request (measured: 344 to 767 tokens with the four
/// models) and the answer (400 at the most), with room to spare
pub const DESCRIBE_CONTEXT: u32 = 4096;
/// GB of a model's working memory for each 8,192 tokens of context: LM Studio, Ollama and llama.cpp (measured
/// with LM Studio's own estimate: 1.5 for Gemma 4 31B, less for the Qwen models; this errs on the large side),
/// and vLLM, which keeps a whole context's worth for each layer (measured: 5.3 GB for Gemma 4 31B at 8,192)
const CONTEXT_GB: f64 = 2.0;
const VLLM_CONTEXT_GB: f64 = 5.5;
/// GB a mixture of experts keeps on the card besides its shared weights when its experts stay in memory: the
/// picture part and the working buffers (measured: 3.1 GB in all for Qwen 3.6 35B-A3B on an RTX 4060, of which
/// about 1.9 its shared weights; this errs on the large side)
const MOE_CARD_GB: f64 = 1.5;

/// GB of working memory for a context on a kind of server.
fn context_gb(platform: &str, context: u32) -> f64 {
    (if platform == "vllm" { VLLM_CONTEXT_GB } else { CONTEXT_GB }) * f64::from(context) / 8192.0
}

/// A model for describing photos, with its name on each kind of server.
#[derive(Debug, PartialEq)]
pub struct Candidate {
    pub id: &'static str,
    pub name: &'static str,
    pub lmstudio: &'static str,
    pub ollama: &'static str,
    /// its file in LM Studio's models folder (llama.cpp uses the same file, with the picture part beside it)
    pub gguf: &'static str,
    pub mmproj: &'static str,
    /// vLLM's copy on Hugging Face (the publisher's own: 4 bits where there is one, else 8)
    pub vllm: &'static str,
    /// GB at 4 bits with its picture part (LM Studio, Ollama, llama.cpp)
    pub gb: f64,
    /// GB of vLLM's copy
    pub vllm_gb: f64,
    /// GB it takes on the graphics processor with a 4,096 context, all of it there (measured: LM Studio's own
    /// estimate, or the card's memory in use while it ran; the picture part and the working buffers included)
    pub card_gb: f64,
    /// billions of parameters used for each word (all of them for a dense model; few for a mixture of experts),
    /// and in all
    pub active_b: f64,
    pub total_b: f64,
}

/// Best first for describing photos (measured: see the guide).
pub const CANDIDATES: [Candidate; 4] = [
    Candidate {
        id: "qwen3.6-35b-a3b",
        name: "Qwen 3.6 35B-A3B",
        lmstudio: "qwen/qwen3.6-35b-a3b",
        ollama: "qwen3.6:35b-a3b",
        gguf: "Qwen3.6-35B-A3B-Q4_K_M.gguf",
        mmproj: "mmproj-Qwen3.6-35B-A3B-BF16.gguf",
        vllm: "Qwen/Qwen3.6-35B-A3B-FP8",
        gb: 22.1,
        vllm_gb: 37.5,
        card_gb: 21.4,
        active_b: 3.0,
        total_b: 35.0,
    },
    Candidate {
        id: "gemma-4-31b",
        name: "Gemma 4 31B",
        lmstudio: "google/gemma-4-31b",
        ollama: "gemma4:31b",
        gguf: "gemma-4-31B-it-Q4_K_M.gguf",
        mmproj: "mmproj-gemma-4-31B-it-BF16.gguf",
        vllm: "google/gemma-4-31B-it-qat-w4a16-ct",
        gb: 19.9,
        vllm_gb: 23.3,
        card_gb: 21.3,
        active_b: 31.0,
        total_b: 31.0,
    },
    Candidate {
        id: "qwen3.8-27b",
        name: "Qwen 3.8 27B",
        lmstudio: "qwen/qwen3.8-27b",
        ollama: "qwen3.8:27b",
        gguf: "Qwen3.8-27B-Q4_K_M.gguf",
        mmproj: "mmproj-Qwen3.8-27B-BF16.gguf",
        vllm: "Qwen/Qwen3.8-27B-FP8",
        gb: 17.7,
        vllm_gb: 30.9,
        card_gb: 19.2,
        active_b: 27.0,
        total_b: 27.0,
    },
    Candidate {
        id: "gemma-4-e4b",
        name: "Gemma 4 E4B",
        lmstudio: "google/gemma-4-e4b",
        ollama: "gemma4:e4b",
        gguf: "gemma-4-E4B-it-Q4_K_M.gguf",
        mmproj: "mmproj-gemma-4-E4B-it-BF16.gguf",
        vllm: "google/gemma-4-E4B-it-qat-w4a16-ct",
        gb: 6.3,
        vllm_gb: 11.5,
        card_gb: 4.3,
        active_b: 4.0,
        total_b: 8.0,
    },
];

impl Candidate {
    /// Its name on a kind of server.
    pub fn on(&self, platform: &str) -> &'static str {
        match platform {
            "ollama" => self.ollama,
            "llamacpp" => self.gguf,
            "vllm" => self.vllm,
            _ => self.lmstudio,
        }
    }

    /// GB it takes on a kind of server.
    pub fn size(&self, platform: &str) -> f64 {
        if platform == "vllm" { self.vllm_gb } else { self.gb }
    }

    /// Whether a server's model is this one: its name on that kind, or (llama.cpp and vLLM, which name a model
    /// as they were started) a name that contains the model's file name or its id.
    fn is(&self, platform: &str, model: &str) -> bool {
        let m = model.to_lowercase();
        let stem = self.gguf.trim_end_matches(".gguf").to_lowercase();
        m == self.on(platform).to_lowercase()
            || m.contains(&stem)
            || m.contains(self.id)
            || m.ends_with(&self.vllm.to_lowercase())
    }
}

/// What a computer can hold for a model, in GB.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    /// the graphics processor's, where it says it
    pub gpu: String,
    /// what a model may take on the graphics processor
    pub gpu_gb: f64,
    /// what may hold the rest of a model in the computer's memory
    pub ram_gb: f64,
    pub separate: bool,
}

/// What this computer can hold: its largest graphics processor's memory less what is kept for the rest.
pub fn room(gpus: &[GpuMemory], memory_gb: f64) -> Room {
    let best = gpus.iter().max_by(|a, b| (a.own_gb + a.shared_gb).total_cmp(&(b.own_gb + b.shared_gb)));
    match best {
        Some(g) if g.shared_gb > 0.0 => Room {
            gpu: g.name.clone(),
            gpu_gb: (g.own_gb + g.shared_gb - KEEP_SHARED).max(0.0),
            ram_gb: (memory_gb / 2.0 - KEEP_SHARED).max(0.0),
            separate: false,
        },
        Some(g) => Room {
            gpu: g.name.clone(),
            gpu_gb: (g.own_gb - KEEP_SEPARATE).max(0.0),
            ram_gb: (memory_gb / 2.0).max(0.0),
            separate: true,
        },
        None => Room { gpu: String::new(), gpu_gb: 0.0, ram_gb: (memory_gb / 2.0).max(0.0), separate: false },
    }
}

/// This computer's room.
pub fn here() -> Room {
    room(&crate::hardware::gpu_memory(), crate::hardware::memory_gb().unwrap_or(0) as f64)
}

/// A model and its settings for a kind of server on a computer.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub id: &'static str,
    pub name: &'static str,
    /// its name on the server
    pub model: String,
    /// whether the server has it already
    pub there: bool,
    pub context: u32,
    /// the part of it on the graphics processor, 0.0 to 1.0
    pub gpu_share: f64,
    /// GB it needs with its context
    pub need_gb: f64,
    /// what to set, as (what, value), for the page
    pub settings: Vec<(String, String)>,
    /// how to start the server with it (llama.cpp, vLLM), or how to load it (LM Studio, Ollama)
    pub command: String,
    pub why: String,
}

/// How much of a model fits on the graphics processor (1.0: all), or None where too little of it would.
fn share(c: &Candidate, platform: &str, room: &Room, context: u32) -> Option<f64> {
    let gb = c.size(platform);
    let context_gb = context_gb(platform, context);
    // (vLLM: its copy and its own context; the others: what was measured at 4,096, and more context beyond it)
    let need =
        if platform == "vllm" { gb + context_gb } else { c.card_gb + context_gb - self::context_gb(platform, 4096) };
    if need <= room.gpu_gb {
        return Some(1.0);
    }
    // vLLM: the whole model on the card or nothing (measured on an 8 GB card: its 4-bit copies were put on the
    // card whole whatever part was to stay in memory, and it ran out of the card's memory)
    if platform == "vllm" {
        return None;
    }
    // A part on the graphics processor and the rest in memory: only for a mixture of experts, which uses few of
    // its weights a word (a few of many: Gemma 4 E4B uses half of its weights, and is no such model). Its experts stay in memory and the rest goes on the card: the shared part (about its
    // active share of the weights), the picture part and the working buffers (MOE_CARD_GB), and the context.
    // Measured on an 8 GB RTX 4060 with Qwen 3.6 35B-A3B: 3.1 GB on the card, 6 to 9 s a picture.
    if c.active_b > 6.0 || c.total_b < 4.0 * c.active_b {
        return None;
    }
    let card = gb * c.active_b / c.total_b + MOE_CARD_GB + context_gb;
    let fits = card <= room.gpu_gb && gb <= room.ram_gb;
    fits.then(|| ((room.gpu_gb - context_gb) / gb).clamp(card / gb, 0.99))
}

/// The settings of a model on a kind of server, and the command that starts or loads it.
fn settings(
    platform: &str,
    c: &Candidate,
    part: f64,
    context: u32,
    models_dir: &Path,
) -> (Vec<(String, String)>, String) {
    let all = part >= 1.0;
    let percent = format!("{:.0} %", part * 100.0);
    let mut s = vec![("Context".to_string(), context.to_string()), ("Requests at a time".to_string(), "1".to_string())];
    let command = match platform {
        "ollama" => {
            s.push((
                "On the graphics processor".to_string(),
                if all { "all of it".to_string() } else { format!("{percent} (Ollama splits it by itself)") },
            ));
            s.push(("Kept loaded".to_string(), "30 minutes after the last photo".to_string()));
            s.push((
                "Ollama's own settings".to_string(),
                "OLLAMA_FLASH_ATTENTION=1, OLLAMA_NUM_PARALLEL=1".to_string(),
            ));
            format!("ollama pull {}   (then the app loads it with num_ctx {context})", c.ollama)
        }
        "llamacpp" => {
            let dir = models_dir.join("lmstudio-community");
            let folder = c.gguf.trim_end_matches(".gguf").trim_end_matches("-Q4_K_M").to_string() + "-GGUF";
            let model = dir.join(&folder).join(c.gguf);
            let mmproj = dir.join(&folder).join(c.mmproj);
            // a mixture of experts keeps its experts in memory and the rest on the graphics processor
            let place = if all { "-ngl 999".to_string() } else { "-ngl 999 --n-cpu-moe 99".to_string() };
            s.push((
                "Layers on the graphics processor".to_string(),
                if all { "all (-ngl 999)".to_string() } else { "all but the experts (--n-cpu-moe)".to_string() },
            ));
            s.push(("Flash attention".to_string(), "on (-fa on)".to_string()));
            format!(
                "llama-server -m \"{}\" --mmproj \"{}\" -c {context} {place} -fa on --parallel 1 --host 0.0.0.0 --port 8080 --alias {}",
                model.display(),
                mmproj.display(),
                c.id
            )
        }
        // (vLLM only ever with the whole model on the card: `share`)
        "vllm" => {
            s.push(("Graphics memory for vLLM".to_string(), "85 % (--gpu-memory-utilization 0.85)".to_string()));
            format!(
                "vllm serve {} --served-model-name {} --max-model-len {context} --max-num-seqs 1 --gpu-memory-utilization 0.85 --limit-mm-per-prompt '{{\"image\":1}}' --host 0.0.0.0 --port 8000",
                c.vllm, c.id
            )
        }
        _ => {
            let gpu = if all { "max".to_string() } else { format!("{:.2}", part) };
            s.push((
                "On the graphics processor".to_string(),
                if all { "all of it (max)".to_string() } else { percent },
            ));
            s.push(("Unloaded after".to_string(), "an hour without use".to_string()));
            format!("lms load {} --context-length {context} --gpu {gpu} --parallel 1 --ttl 3600", c.lmstudio)
        }
    };
    (s, command)
}

/// The best model for describing photos with this kind of server on a computer with this room, and its
/// settings. `have`: the models the server has (none known: an empty list). None where no model would run.
pub fn plan(platform: &str, room: &Room, have: &[String], context: u32, models_dir: &Path) -> Option<Plan> {
    let fits = |c: &Candidate| share(c, platform, room, context).filter(|p| *p >= 1.0).is_some();
    let has = |c: &Candidate| have.iter().find(|m| c.is(platform, m)).cloned();
    // What the server has comes first (it runs now, nothing to get): the best of it that fits wholly, else the
    // best of it that fits partly; else the best model that fits wholly, else partly, to get for it
    let partly = |c: &Candidate| share(c, platform, room, context).is_some();
    let chosen = CANDIDATES
        .iter()
        .find(|c| fits(c) && has(c).is_some())
        .or_else(|| CANDIDATES.iter().find(|c| partly(c) && has(c).is_some()))
        .or_else(|| CANDIDATES.iter().find(|c| fits(c)))
        .or_else(|| CANDIDATES.iter().find(|c| partly(c)))?;
    let part = share(chosen, platform, room, context)?;
    let there = has(chosen);
    let (settings, command) = settings(platform, chosen, part, context, models_dir);
    let need = if platform == "vllm" {
        chosen.vllm_gb + context_gb(platform, context)
    } else {
        chosen.card_gb + context_gb(platform, context) - context_gb(platform, 4096)
    };
    let mut why = if part >= 1.0 {
        format!(
            "{} needs about {need:.0} GB and {} can take {:.0} GB for it: all of it runs there.",
            chosen.name,
            if room.gpu.is_empty() { "the graphics processor".to_string() } else { room.gpu.clone() },
            room.gpu_gb
        )
    } else {
        format!(
            "{} needs about {need:.0} GB; {:.0} GB fit on the graphics processor, and the rest runs from the computer's memory (it uses few of its weights a word, so that stays quick).",
            chosen.name, room.gpu_gb
        )
    };
    // a better one that the server doesn't have
    if let Some(better) = CANDIDATES.iter().take_while(|c| *c != chosen).find(|c| fits(c)) {
        why += &format!(" {} would be better and fits: get it for this server.", better.name);
    }
    if there.is_none() {
        why += &format!(" The server doesn't have it yet: get {} for it.", chosen.on(platform));
    }
    Some(Plan {
        id: chosen.id,
        name: chosen.name,
        model: there.clone().unwrap_or_else(|| chosen.on(platform).to_string()),
        there: there.is_some(),
        context,
        gpu_share: part,
        need_gb: (need * 10.0).round() / 10.0,
        settings,
        command,
        why,
    })
}

/// Why vLLM can't serve these models on a computer, where it can't: it runs on Linux with an NVIDIA card, and
/// needs the whole model on that card (measured: Gemma 4 31B's 4-bit copy and Gemma 4 E4B's both ran out of an
/// 8 GB card; Qwen 3.6 35B-A3B's 8-bit copy is 37.5 GB).
pub fn vllm_cannot(platform: &str, room: &Room) -> Option<String> {
    if platform != "vllm" {
        return None;
    }
    if !cfg!(target_os = "linux") || !room.gpu.to_lowercase().contains("nvidia") {
        return Some("vLLM runs on Linux with an NVIDIA card, and this computer has neither: use LM Studio, Ollama or llama.cpp here.".to_string());
    }
    let smallest = CANDIDATES.iter().map(|c| c.vllm_gb + context_gb("vllm", DESCRIBE_CONTEXT)).fold(f64::MAX, f64::min);
    (smallest > room.gpu_gb).then(|| {
        format!(
            "vLLM needs the whole model on the card, and this card can take {:.0} GB of the {smallest:.0} GB the smallest of these models needs with vLLM: use LM Studio, Ollama or llama.cpp here, which can keep part of a model in the computer's memory.",
            room.gpu_gb
        )
    })
}

/// The plan for the describer's server as it is set, for the page: {for, plan} or {for, why} (a server on
/// another device is suggested for in that device's own Settings: its hardware is its own).
pub fn suggestion(c: &Config, have: &[String]) -> Value {
    if where_is(&c.url).0 != "this computer" {
        return json!({"here": false, "why": "The server is on another device: its model and settings are suggested in that device's own Settings, for its own hardware."});
    }
    let room = here();
    if let Some(why) = vllm_cannot(&c.platform, &room) {
        return json!({"here": true, "why": why});
    }
    match plan(&c.platform, &room, have, DESCRIBE_CONTEXT, &crate::lmstudio::models_dir()) {
        Some(p) => {
            json!({"here": true, "gpu": room.gpu, "gpu_gb": (room.gpu_gb * 10.0).round() / 10.0, "model": p.model, "name": p.name, "there": p.there,
                          "context": p.context, "gpu_share": (p.gpu_share * 100.0).round() / 100.0, "need_gb": p.need_gb,
                          "settings": p.settings.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(), "command": p.command, "why": p.why})
        }
        None => {
            json!({"here": true, "why": "This computer has too little graphics memory for any of the models that describe photos well: use a server on another device, or the built-in model."})
        }
    }
}

/// Load the plan's model on the describer's server, with its settings. LM Studio only on this computer (its
/// own tool does it), Ollama wherever it is (a request that loads it). What was done, or why not.
pub fn load(c: &Config, p: &Plan) -> Result<String, String> {
    if !p.there {
        return Err(format!(
            "The server doesn't have {} yet: get it first (Settings never downloads one by itself).",
            p.model
        ));
    }
    match c.platform.as_str() {
        "lmstudio" => {
            if where_is(&c.url).0 != "this computer" {
                return Err("LM Studio on another device loads its model there: in that device's Settings.".to_string());
            }
            if loaded_lmstudio(c, &p.model, p.context) {
                return Ok(format!("{} is loaded already, with a context of {} or more.", p.model, p.context));
            }
            let gpu = if p.gpu_share >= 1.0 { "max".to_string() } else { format!("{:.2}", p.gpu_share) };
            let context = p.context.to_string();
            let args = [
                "load",
                p.model.as_str(),
                "--context-length",
                &context,
                "--gpu",
                &gpu,
                "--parallel",
                "1",
                "--ttl",
                "3600",
                "--yes",
            ];
            match crate::lmstudio::lms(&args, Duration::from_secs(600)) {
                Some((true, _)) => Ok(format!(
                    "{} loaded: context {}, {} on the graphics processor.",
                    p.model,
                    p.context,
                    if p.gpu_share >= 1.0 { "all of it".to_string() } else { format!("{:.0} %", p.gpu_share * 100.0) }
                )),
                Some((false, said)) => Err(format!(
                    "LM Studio couldn't load it: {}",
                    said.lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                None => {
                    Err("LM Studio's lms tool isn't on this computer, or didn't answer in ten minutes.".to_string())
                }
            }
        }
        "ollama" => {
            let body = json!({"model": p.model, "keep_alive": "30m", "options": {"num_ctx": p.context}});
            crate::describe::post_for_load(&c.url, "/api/generate", &c.key, &body).map(|()| {
                format!(
                    "{} loaded in Ollama with a context of {}; it stays loaded 30 minutes after the last photo.",
                    p.model, p.context
                )
            })
        }
        _ => Err(format!(
            "{} serves the model it was started with: start it with the command shown.",
            if c.platform == "vllm" { "vLLM" } else { "llama.cpp" }
        )),
    }
}

/// Whether LM Studio has this model loaded with at least this context.
fn loaded_lmstudio(c: &Config, model: &str, context: u32) -> bool {
    crate::describe::lmstudio_loaded(&c.url, &c.key)
        .iter()
        .any(|(m, ctx)| m == model && ctx.is_none_or(|n| n >= u64::from(context)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, own: f64, shared: f64) -> GpuMemory {
        GpuMemory { name: name.to_string(), own_gb: own, shared_gb: shared }
    }

    #[test]
    fn what_a_computer_can_hold_for_a_model() {
        // a separate 8 GB card: its own memory less what the app keeps
        let card = room(&[gpu("NVIDIA GeForce RTX 4060 Laptop GPU", 8.0, 0.0)], 61.0);
        assert_eq!((card.gpu_gb, card.ram_gb, card.separate), (6.5, 30.5, true));
        // a processor's built-in graphics with 64 GB set aside and half of 64 GB shared
        let apu = room(&[gpu("AMD Radeon(TM) 8060S Graphics", 64.0, 32.0)], 64.0);
        assert_eq!((apu.gpu_gb, apu.separate), (88.0, false));
        // nothing that says its memory: only the processor
        assert_eq!(room(&[], 16.0).gpu_gb, 0.0);
    }

    #[test]
    fn the_best_model_the_server_has_that_fits_with_its_settings() {
        let dir = Path::new("models");
        let big = room(&[gpu("AMD Radeon(TM) 8060S Graphics", 64.0, 32.0)], 64.0);
        let have = vec!["qwen/qwen3.8-27b".to_string(), "google/gemma-4-31b".to_string()];
        let p = plan("lmstudio", &big, &have, DESCRIBE_CONTEXT, dir).unwrap();
        // the best of what LM Studio has, all on the graphics processor; a better one it lacks is named
        assert_eq!((p.id, p.there, p.gpu_share), (CANDIDATES[1].id, true, 1.0));
        assert!(p.why.contains(CANDIDATES[0].name), "{}", p.why);
        assert_eq!(p.command, "lms load google/gemma-4-31b --context-length 4096 --gpu max --parallel 1 --ttl 3600");
        // an 8 GB card takes the small model wholly (measured: 4.3 GB, a picture a second), and a model that fits
        // wholly comes before one split with the memory
        let card = room(&[gpu("NVIDIA GeForce RTX 4060 Laptop GPU", 8.0, 0.0)], 61.0);
        let p = plan("llamacpp", &card, &[], DESCRIBE_CONTEXT, dir).unwrap();
        assert_eq!((p.id, p.gpu_share), ("gemma-4-e4b", 1.0), "{p:?}");
        assert!(
            p.command.contains("-ngl 999")
                && p.command.contains("-c 4096")
                && p.command.contains("--alias gemma-4-e4b")
        );
        // the same card on a server that has only the mixture of experts: that, its experts in memory and the rest on
        // the card (measured: 6 to 9 s a picture), rather than a model to get first
        let p = plan("llamacpp", &card, &["Qwen3.6-35B-A3B-Q4_K_M".to_string()], DESCRIBE_CONTEXT, dir).unwrap();
        assert_eq!((p.id, p.there), ("qwen3.6-35b-a3b", true), "{p:?}");
        assert!(p.gpu_share < 1.0 && p.command.contains("-ngl 999 --n-cpu-moe 99"), "{p:?}");
        // too little memory for the experts as well: no split, and nothing that fits
        let small = room(&[gpu("NVIDIA GeForce GTX 1650", 4.0, 0.0)], 16.0);
        assert!(plan("llamacpp", &small, &[], DESCRIBE_CONTEXT, dir).is_none());
        // vLLM on the same card: its copies are larger, it keeps more for the context, and it takes a model whole
        // or not at all: none of them, and Settings says why
        assert!(plan("vllm", &card, &[], DESCRIBE_CONTEXT, dir).is_none());
        if cfg!(target_os = "linux") {
            assert!(vllm_cannot("vllm", &card).unwrap().contains("whole model on the card"));
        }
        assert!(vllm_cannot("vllm", &big).is_some_and(|why| why.contains("NVIDIA")), "a Radeon is no card for vLLM");
        assert!(vllm_cannot("ollama", &card).is_none());
        // a card that takes Gemma 4 31B's copy whole
        let large = room(&[gpu("NVIDIA RTX A6000", 48.0, 0.0)], 128.0);
        let p = plan("vllm", &large, &[], DESCRIBE_CONTEXT, dir).unwrap();
        assert!(
            p.gpu_share == 1.0 && p.command.contains("--max-model-len 4096") && !p.command.contains("offload"),
            "{p:?}"
        );
        // a card too small for any of them
        assert!(
            plan("vllm", &room(&[gpu("NVIDIA GeForce GTX 1050", 4.0, 0.0)], 32.0), &[], DESCRIBE_CONTEXT, dir)
                .is_none()
        );
        // Ollama's names, and what it has
        let p = plan("ollama", &big, &["gemma4:31b".to_string()], DESCRIBE_CONTEXT, dir).unwrap();
        assert_eq!((p.model.as_str(), p.there), ("gemma4:31b", true));
        // too little for anything
        assert!(plan("lmstudio", &room(&[], 8.0), &[], DESCRIBE_CONTEXT, dir).is_none());
    }

    #[test]
    fn a_servers_model_is_known_by_its_name_on_that_kind() {
        let g = &CANDIDATES[1];
        assert!(g.is("lmstudio", "google/gemma-4-31b") && g.is("ollama", "gemma4:31b"));
        assert!(g.is("llamacpp", "gemma-4-31B-it-Q4_K_M") && g.is("llamacpp", "gemma-4-31b"));
        assert!(g.is("vllm", "google/gemma-4-31B-it-qat-w4a16-ct"));
        assert_eq!((g.size("vllm"), g.size("ollama")), (23.3, 19.9));
        assert!(!g.is("lmstudio", "qwen/qwen3.8-27b"));
    }
}
