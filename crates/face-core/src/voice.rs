//! The voice model (how aroused, dominant and pleasant a stretch of speech sounds): Melos 1, the project's own
//! (`models/melos_1.onnx`, one of `face-tools`' own models), trained by the project on what listeners heard in
//! the clips of CREMA-D (Open Database License). Optional: without the file there is none, and the tracker says
//! so. Evidence only, like every signal beside the face: it was measured on acted speech, and what a voice
//! sounds like is not what a person feels. The sound is scored and never kept.

use std::path::Path;

use crate::{
    Error, Result,
    runtime::{Model, Runtime},
};

/// samples a second the model expects
pub const RATE: usize = 16_000;

pub struct Voice {
    model: Option<Model>,
}

impl Voice {
    /// The model on the processor, where it was measured; none when its file isn't there.
    pub fn load(runtime: &Runtime, models: &Path) -> Voice {
        let path = models.join("melos_1.onnx");
        let model = path.is_file().then(|| runtime.model_on_cpu(&path, "voice")).and_then(|m| match m {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("The voice model didn't load: {}", e.0);
                None
            }
        });
        Voice { model }
    }

    pub fn loaded(&self) -> bool {
        self.model.is_some()
    }

    /// [arousal, dominance, valence] for samples at `RATE`, or None without the model.
    pub fn avd(&self, samples: &[f32]) -> Result<Option<[f32; 3]>> {
        let Some(model) = &self.model else { return Ok(None) };
        let out = model.run(&[1, samples.len()], samples)?;
        match out.first().map(Vec::as_slice) {
            Some([a, d, v, ..]) => Ok(Some([*a, *d, *v])),
            _ => Err(Error::msg("the voice model gave an unexpected answer")),
        }
    }
}
