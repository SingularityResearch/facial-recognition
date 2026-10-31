# Real-Time Facial Emotion Recognition
Model: qwen/qwen3.8-27b
Created: 10/28/2026, 10:04:57 PM
Exported from: LM Studio 0.4.26+4

### User
Describe the best approaches to identifying emotions in real time streams of live human faces

### Assistant
We need answer user: "Describe the best approaches to identifying emotions in real time streams of live human faces". Need provide thorough answer. Need think. User likely asks for best approaches. Need discuss real-time emotion recognition from live face video streams. We should cover caveats: ethical, consent, limitations, privacy, accuracy, cultural/context. Techniques: face detection/alignment/tracking, feature extraction: 3D geometry, facial action units, optical flow, deep CNN/Transformer, video models, temporal modeling, multi-modal context. Real-time constraints: latency, throughput, edge/cloud, model optimization, streaming, asynchronous, Kalman smoothing, ensemble. Datasets, training, evaluation. Best practice pipeline. Discuss AU-based, Deep learning, hybrid, multimodal (speech/body), LLM? Emotion identification: discrete vs dimensional (valence/arousal). Real time streams: frame sampling, tracking, temporal aggregation, event detection, confidence calibration. Mention open source: OpenCV, MediaPipe, dlib, InsightFace, DeepFace, OpenFace, FER, AffectNet, Aff-Wild, RAVDESS? Emotion models: CNN (ResNet, MobileNet, EfficientNet, Vision Transformer), 3D CNN, I3D, SlowFast, VideoMAE, X3D, temporal convolution, LSTM/Transformer, pose/landmarks + MLP. AU recognition: OpenFace 2.36 AU, FaceReader, Affectiva? Commercial? Need maybe not specific. Discuss best approaches: (1) AU-based interpretable, (2) deep video CNN with landmarks/depth, (3) hybrid: face alignment + AU + deep features + temporal smoothing, (4) multi-modal. Real-time: edge deployment, quantization, ONNX/TensorRT, batch small, asynchronous, ROI, skip frames, confidence.

Need maybe answer: "best approaches" not "code". Provide architecture. Maybe mention no single best, depends on accuracy vs latency vs interpretability. For live human faces, best practical is: track faces, extract dense 2D/3D landmarks/AUs, use lightweight temporal deep model to predict valence/arousal/discrete categories, calibrate and smooth, optionally fuse audio/body. Use dimensional output + top categories. Need include robustness: pose, expression, ethnicity, lighting, occlusion, glasses, camera quality, self-report ground truth.

Need ensure not encourage covert surveillance or manipulation. Add ethics: consent, transparency, minimize data, local processing, avoid high-stakes uses, bias, legal (GDPR, BIPA maybe). Since live streams, privacy.

Need maybe mention "identifying emotions" can be mislabeled; better estimate affective state. Emotions not directly observable from faces; faces can indicate emotional expressions, not internal states. So phrase: estimate facial expressions/affectionate states.

Structure:
1. Clarify goal and limits.
2. High-level pipeline.
3. Approaches ranked:
   A. Landmark/AU + temporal classifier (interpretable, robust, low data). Use 3DMM, OpenFace/MediaPipe landmarks, compute AUs (AU1, AU2, AU4, AU5, AU6, AU7, AU12, AU15, AU17, AU20, etc). Temporal smoothing (LSTM/TCN/Transformer). Predict FACS-based discrete or valence/arousal. Good for real-time.
   B. End-to-end deep video model: crops, 3D CNN/Video Transformer; pretrain on expression datasets; efficient variants (MobileNetV3, EfficientNet-Lite, SlowFast-lite, TimeSformer); fine-tune; temporal pooling; optional depth.
   C. Hybrid ensemble: deep features + AUs + head pose + gaze + blink; fusion late/early; calibrate; robust and interpretable.
   D. Multimodal: speech prosody, body pose, text/context, heart rate if wearable.
   E. 3D/geometry: 3DMM, FLAME, depth/IR (if available), occlusion-invariant.
4. Real-time implementation details:
- Face detection/tracking: MediaPipe/RetinaFace/YOLO-Face; persistent track IDs; alignment to canonical; handle multiple faces.
- Sampling and temporal window: 10-30 fps; window 0.5-2s; stride 0.1-0.5s; smoothing EMA; confidence threshold; hysteresis.
- Model optimization: quantization, pruning, distillation, ONNX/TensorRT/NCNN/CoreML; async inference; dynamic resolution; edge vs cloud; latency budget (&lt;100-300ms); resource.
- Calibration and uncertainty: temperature scaling, Bayesian, ensembles; output probabilities/dimensions; abstain low confidence.
- Evaluation: frame-level, event-level, macro-F1, F1 per class, accuracy valence/arousal; bias by age/ethnicity/gender; cross-culture; temporal stability; latency.
- Data: AffectNet, Aff-Wild II/III, RAF-DB, FER2013, MultiPIE? Self-collected; label with self-report; avoid FER2013 too simplistic.
- Context: use user feedback, adaptive personalization; domain adaptation; few-shot calibration; active learning.
5. Best practice recommendation: Start with lightweight hybrid: MediaPipe landmarks + 3DMM + AU features + temporal TCN/LSTM + valence/arousal + top 3 discrete; optional deep vision encoder; deploy on edge; collect feedback.
6. Pitfalls: static frames unreliable, single expression class, culture, privacy, bias, overclaiming, adversarial, occlusion.
7. Example architecture/pseudo pipeline.
8. If live human faces in real time, use streaming state: track, sliding windows, Kalman/EMA, event trigger.
9. Mention "emotions" vs "expressions".

Need answer likely not too code. But could include pseudo code? User asks describe best approaches. Could include bullet list and recommendation. Need be comprehensive.

Need think about any policy? No. Could mention sensitive uses. Good.

Need final with headings.

Need maybe include specific model names:
- Face detection: BlazeFace/MediaPipe, RetinaFace, SCRFD, YOLOv8-face, MTCNN.
- Landmarks: MediaPipe Face Mesh (468 points), dlib 68, OpenFace, 3DMM/FLAME, FaceAnalysis.
- AU: OpenFace, Affectiva (commercial), FaceReader, AU-Nets, DeepAU.
- Emotion classifiers: ResNet-50, EfficientNet-B0/B3, MobileNetV3-Small, ConvNeXt-Tiny, ViT-S, SlowFast R50, I3D, X3D, TimeSformer, VideoMAE. For low latency: MobileNetV3, EfficientNet-Lite, ConvNeXt-F, lightweight 3D CNN.
- Temporal: LSTM, GRU, TCN, Transformer (Swin?), attention, Kalman smoother.
- Multimodal: Wav2Vec/HuBERT, Whisper prosody? body pose (OpenPose), heart rate (PPG if camera), text.

Need mention dimensional (valence/arousal) often better than discrete. Discrete: happy, sad, angry, fear, surprise, disgust, neutral. Some systems predict 7/8 classes. But real time better to output continuous valence/arousal plus confidence and then map to labels.

Need maybe discuss real-time streams of live human faces: could be multiple faces, video call, classroom, retail? Need privacy. For multiple faces, track and label each.

Need mention "best approaches" maybe includes "active appearance models"? Historical: GMM, HMM, SVM, CNN, RNN. Now deep learning. But best practical: deep learning with temporal modeling.

Need maybe discuss "face geometry + optical flow". Optical flow can capture dynamics (eyebrow raise, mouth open) but expensive. Landmarks dynamics equivalent. 3DMM parameters are robust and compact.

Need maybe mention "3D face reconstruction" FLAME, OFM (OpenFace), Basel Face Model. Use 68/468 landmarks + pose + AUs. Real-time: MediaPipe FaceMesh gives 3D landmarks and blendshapes; can map to AUs. Use blendshapes directly as expression features. Good.

Need maybe mention "AU dynamic vs static". AUs: intensity, onset/offset; temporal patterns important. Use microexpressions? Not reliable in real time.

Need maybe mention "deep learning for AU" vs direct emotion. Direct deep may be less interpretable but better if large data. Hybrid can be more robust and fair.

Need maybe mention "calibration per person"? Emotion expression varies. Use self-supervised personal adaptation.

Need maybe discuss "latency and throughput". E.g. target 15-30 fps for natural; inference 10-50 ms per face on GPU; 100-300 ms end-to-end. Use asynchronous queue; face crop at 112x112/160x160; batch; dynamic batching; reduce ROI; use half precision; TFLite/NCNN for mobile. Use face tracking to avoid full detection every frame; only update detection periodically.

Need mention "state estimation" over time: use hidden Markov model? Or Bayesian filter. Smoothing reduces jitter but increases latency. Use causal filters for real-time (no future frames). E.g., exponential moving average with alpha 0.2-0.5; Kalman with observation uncertainty; HMM/Viterbi for event labels; event trigger with dwell time.

Need mention "uncertainty and abstention": if low confidence, neutral, no label; confidence via entropy, temperature scaling; output probability distribution.

Need mention "ethical constraints": explicit consent, no deception, minimize retention, local inference, anonymization, audit for bias, do not use for employment, policing, immigration, credit, etc. In high-stakes, human oversight. Data minimization, DPIA.

Need maybe mention "regulatory: GDPR, CCPA, Illinois BIPA (biometric identifiers), EU AI Act high-risk? Emotion recognition in workplace/education banned/regulated in some jurisdictions." Need careful: mention check local law.

Need final should be maybe long, with actionable approach.

Let's craft final:
- Opening: The best approach depends on constraints; no model can read emotions directly, estimate facial expressions/affective cues.
- Recommended pipeline:
  1. Capture & face tracking
  2. Expression representation: 2D/3D landmarks, AUs, blendshapes, depth, optional deep embeddings
  3. Temporal model
  4. Output: valence/arousal + discrete labels + confidence
  5. Smoothing/event detection
  6. Calibration/feedback
- Then compare approaches:
  A. Landmark/AU based: pros/cons, best when interpretability, low data, edge.
  B. Deep visual video models: pros/cons, best when accuracy and large data.
  C. Hybrid (best overall): combine.
  D. Multimodal: best for accuracy/context.
  E. 3D geometry: robust to pose/lighting.
- Real-time specifics:
  - Face detection/tracking: MediaPipe, RetinaFace, SCRFD; track IDs; alignment; handle occlusion.
  - Feature extraction: 468 landmarks, 3DMM/FLAME, AUs, head pose, gaze, blink, mouth openness, optical flow optional.
  - Model choices: MobileNetV3/EfficientNet-Lite/ConvNeXt-F + TCN/LSTM/Transformer; SlowFast-lite/X3D if GPU; 3D CNN; ensemble.
  - Inference: 10-30 fps; window 0.5-2s; stride; causal; quantization; TensorRT; edge.
  - Temporal smoothing: EMA/Kalman/HMM; dwell time; hysteresis; confidence.
- Data/training/eval:
  - datasets: AffectNet, Aff-Wild II/III, RAF-DB, MultiPIE, SEED, M-ERD? self-collected; labels self-report; bias.
  - metrics: macro-F1, per class, valence/arousal MSE, correlation, AUC, temporal stability, latency, fairness.
  - domain adaptation: few-shot, active learning, user feedback.
- Pitfalls:
  - single frame, universal emotion assumption, culture, pose, lighting, ethnicity/gender bias, privacy, manipulation, overclaiming.
- Recommended starting stack:
  - MediaPipe FaceMesh + 3DMM/FLAME or AUs + small TCN/LSTM + valence/arousal + top labels, local processing, consent, evaluation. Add deep encoder if accuracy needed.
- Maybe include pseudo code:
  for each video frame: detect/track faces -&gt; crop/align -&gt; extract landmarks/AUs -&gt; append to window -&gt; inference -&gt; smooth -&gt; emit event.
- Conclude.

Need ensure no false claims about MediaPipe blendshapes? MediaPipe FaceMesh includes 52 blendshapes? It can output face blendshapes? I think MediaPipe Face Landmarker includes 52 blendshapes. It's okay.

Need mention "AUs: FACS" and examples. Use FACS as standard.

Potential nuance: "real-time streams of live human faces" could be used for interactive systems. We can mention "if the stream is high-resolution, use 3D face reconstruction; if low-res, use deep appearance features". Need mention "multiple faces and identity" for tracking.

Need maybe include "best approaches to identifying emotions" from a technical perspective, not just code. Good.

Need be careful: "emotions" not directly identifiable. State "facial expression affective states".

Need maybe mention "use dimension-based model as primary because discrete labels are ambiguous and culturally variable". Good.

Let's craft with high detail. Use headings.

Need maybe include table comparing approaches. Good.

Need maybe include "real-time constraints: end-to-end latency, frame rate, memory, power". Provide numbers: target &lt;200 ms for interactive; 15-30 fps; 112-224 px crop; 1-5 ms per frame on modern GPU? Need not too exact but examples.

Need include "asynchronous inference" and "only infer every N frames or on face stable". Good.

Need mention "face alignment and normalization: pose, scale, crop; remove camera motion; canonical face". Good.

Need mention "robustness: occlusion, glasses, beard, makeup, age, ethnicity; use multi-ethnicity data and fairness testing". Good.

Need maybe mention "micro-expressions": not reliable in real-time and can be misleading; avoid as core.

Need include "privacy by design: process locally, don't store raw video, anonymize track IDs, retention policy". Good.

Let's write final with enough detail.

Need final no citations? Could mention dataset names but no references. Fine.

Double-check if any policy about biometric emotion recognition? Could be sensitive. We can provide but with ethical constraints. Not prohibited. Good.

Need perhaps include "best approaches" in order:
1. Hybrid deep+AU is best overall.
2. If edge, landmark-based.
3. If server GPU, video Transformer/3D CNN.
4. If maximum accuracy, multimodal.

Need mention "use causal temporal model only, no future frames".

Need mention "temporal context is crucial because emotions evolve over 0.5-5s". Good.

Need maybe include "action units vs deep features". Good.

Let's draft.

Potential final:

"Short answer: use a streaming face tracker to extract 2D/3D landmarks/blendshapes/AUs and a lightweight temporal deep model that outputs valence/arousal plus discrete labels, with confidence smoothing and optional deep visual encoder. Avoid single-frame classifiers."

Then sections.

Need maybe include "example features: AU5 brow raise, AU12 lip corners, AU17 chin, AU20 strait mouth, eye area, mouth openness, blink rate, head pose, gaze, temporal deltas". Good.

Need mention "pretrained on large expression datasets then fine-tuned on target domain".

Need include "if camera has depth/IR (RealSense, FaceID), use 3D geometry for pose-invariant expressions".

Need maybe include "model fusion: early/late; ensemble weighted by confidence; calibration".

Need include "use self-reported ground truth; avoid using labels inferred from facial expression alone". Good.

Need maybe include "active learning: collect low-confidence clips for labeling". Good.

Need maybe include "real-time event detection: trigger when valence/arousal crosses thresholds for N ms; dwell time; hysteresis". Good.

Need maybe include "multi-face: assign stable IDs, avoid identity inference unless needed; track per face; output per face".

Need maybe include "latency budget example: 33ms frame, detection 2ms, landmarks 3ms, model 5-15ms, smoothing 0, network 20-50ms; total 50-150ms edge". Good.

Need maybe mention "use half precision/INT8; model size &lt;10MB for mobile". Good.

Need maybe mention "avoid heavy optical flow unless necessary".

Need maybe mention "if using audio: extract MFCC, F0, energy, jitter; use Wav2Vec; fuse late". Good.

Need final concise but detailed. Since desired oververbosity 9, produce long.

Let's compose final with headings, table, recommendation.

The best real-time approach is usually **not** a single “emotion detector” running on one frame, but a **streaming pipeline** that tracks faces, extracts robust expression cues, models expression over time, and outputs a calibrated affective estimate with uncertainty. In practice, the strongest systems combine:

1. **Face detection / tracking**
2. **2D/3D facial geometry or AUs** — e.g., landmarks, blendshapes, Action Units
3. **A lightweight deep visual model** — for appearance and texture cues
4. **A temporal model** — because expressions evolve over hundreds of milliseconds to seconds
5. **Smoothed, confidence-gated outputs** — valence/arousal plus optional discrete labels
6. **Ethical safeguards** — consent, privacy, bias testing, and limited use

A key caveat: faces do not directly reveal “emotions” as internal states. A face stream can more reliably estimate **facial expressions and affective signals** — e.g., happiness, sadness, anger, fear, surprise, disgust, confusion, frustration, amusement, boredom — but these should be interpreted with context, consent, and uncertainty.

---

## 1. Recommended architecture

### High-level pipeline

```text
Live video
  → Face detection / tracking
  → Face alignment / normalization
  → Expression feature extraction
       - 2D/3D landmarks
       - blendshapes / AUs
       - head pose, gaze, blink rate
       - deep appearance features
  → Temporal modeling
       - TCN / LSTM / GRU / Transformer
  → Output
       - valence
       - arousal
       - discrete probabilities
       - confidence / uncertainty
  → Smoothing / event detection
  → Application output
```

This is better than a single-frame CNN because many expressions are dynamic and ambiguous in isolation.

---

## 2. Best approaches, ranked by practical usefulness

### A. Hybrid AU/landmark + temporal deep model — best overall for most real-time systems

This is often the best balance of:

- accuracy
- interpretability
- robustness
- low latency
- edge deployment
- bias control
- adaptability

Use a face landmark or 3D face model to extract structured expression features, then feed them into a temporal classifier.

Useful features include:

- 468-point or 3D facial landmarks
- blendshapes, if available
- Action Units from FACS:
  - AU1: inner brow raise
  - AU2: outer brow raise
  - AU4: brow lower
  - AU5: lid tight
  - AU6: cheek raise
  - AU7: lid raise
  - AU12: lip corner pull
  - AU15: lip corner depress
  - AU17: chin raise
  - AU20: stretcher
  - mouth openness
  - jaw drop
  - blink rate
  - head pose
  - gaze direction
  - temporal deltas of all of the above

Then use a compact temporal model:

- 1D Temporal Convolutional Network
- LSTM / GRU
- small Transformer over temporal feature sequence
- MLP with temporal pooling if very constrained

Output:

- continuous valence: negative to positive
- continuous arousal: low to high
- discrete probabilities: happy, sad, angry, fear, surprise, disgust, neutral, confused, etc.

**Why this works well:**

- Action Units are interpretable.
- They generalize better across ethnicity, age, lighting, and camera quality than raw pixels alone.
- They are computationally cheap.
- They reduce reliance on texture alone, which can be biased by skin tone, makeup, lighting, and camera compression.
- Temporal modeling captures dynamics such as eyebrow raise, mouth opening, and expression decay.

**Best for:**

- edge devices
- video calls
- kiosk or classroom settings
- privacy-sensitive deployments
- systems where you need to explain why a state was inferred

---

### B. End-to-end deep video model — best when you need maximum accuracy and have compute

A deep visual model learns appearance, texture, and motion directly from face crops.

Good model families include:

- MobileNetV3
- EfficientNet-Lite
- ConvNeXt-F
- ViT-S / ViT-B, if compute allows
- SlowFast
- I3D
- X3D
- TimeSformer
- VideoMAE
- lightweight 3D CNNs

For real-time systems, the best choices are usually:

- **MobileNetV3 / EfficientNet-Lite / ConvNeXt-F** for appearance
- **TCN or LSTM** for temporal context
- **SlowFast-lite / X3D-lite** only if you have a GPU and enough data

The model can be trained on face crops from datasets such as:

- AffectNet
- Aff-Wild II / Aff-Wild III
- RAF-DB
- MultiPIE
- self-collected domain-specific data

**Advantages:**

- Captures subtle texture and expression cues that AUs may miss.
- Can learn complex non-linear patterns.
- Strong performance when trained on large, well-labeled data.

**Disadvantages:**

- Less interpretable.
- More susceptible to dataset bias.
- Can be sensitive to lighting, ethnicity, age, glasses, occlusion, and camera quality.
- Heavier to deploy on edge devices.
- Requires more data and better calibration.

**Best for:**

- server-side inference
- high-accuracy applications
- environments with consistent camera quality
- research or advanced products where explainability is secondary

---

### C. 3D geometry / 3DMM-based approach — best for robustness to pose and lighting

Use a 3D morphable model, FLAME, or 3D face reconstruction to estimate a canonical facial shape and expression parameters.

This is especially useful when:

- face pose varies a lot
- lighting is unstable
- the camera is low-resolution
- you need stronger generalization
- you want to reduce texture-based bias

You can use 3D parameters to estimate:

- brow height
- brow tilt
- eye openness
- eyelid tension
- mouth corner movement
- lip stretching
- jaw openness
- cheek deformation

A 3DMM/FLAME-based system can be more robust than 2D landmarks because it separates face shape, pose, and expression.

**Best for:**

- real-time applications with variable pose
- multi-person scenes
- robustness across ethnicities and face shapes
- edge deployment if using a lightweight 3DMM

---

### D. Multimodal affect estimation — best when context matters

Faces alone are often insufficient. If the goal is to understand emotional or affective state, the best systems often fuse:

- facial expression
- voice prosody
- speech content
- body posture
- movement dynamics
- context or conversation topic
- wearable signals, if available and consensual

Useful modalities:

- audio: pitch, energy, speaking rate, pauses, intonation
- text: semantic tone, if speech-to-text is available
- body: posture, gesture, stillness, leaning, tension
- physiology: heart rate, skin conductance, if wearables are present
- context: task, conversation, environment

Multimodal fusion can be:

- early fusion: concatenate features before model
- late fusion: combine per-modality predictions
- cross-modal attention: let the model learn interactions

**Best for:**

- virtual assistants
- customer experience analysis
- education or tutoring
- mental wellness or coaching applications, with careful ethics
- high-accuracy research systems

---

## 3. Practical real-time design details

### Face detection and tracking

For live streams, do not re-detect every face from scratch every frame if you can avoid it.

Use:

- MediaPipe Face Detection / Face Mesh
- RetinaFace
- SCRFD
- YOLO-face variants
- BlazeFace
- OpenCV HaarCascade only for very low-end or legacy use

Better pipeline:

1. Detect faces every few frames.
2. Track faces using motion, appearance, or face ID.
3. Assign stable track IDs.
4. Update landmarks and expression features per track.
5. Maintain per-face temporal state.

This improves latency and consistency.

---

### Face normalization

Before expression extraction, normalize the face:

- crop to face ROI
- align eyes and mouth
- scale to canonical size
- correct for head pose if needed
- reduce camera motion effects
- store canonical coordinates

Useful crop sizes:

- 96 × 96
- 112 × 112
- 160 × 160
- 224 × 224

Lower resolution is better for speed; higher resolution is better for subtle expressions.

---

### Temporal modeling

Expressions are time-dependent. A smile, frown, or brow raise may be meaningful only in context.

Use a sliding window over recent frames or extracted features:

- window: 0.5 seconds to 3 seconds
- stride: 100 ms to 500 ms
- model: TCN, LSTM, GRU, or small Transformer

For real-time inference, use only past and present frames. Avoid future-frame models unless you can accept higher latency.

Example temporal features:

- AU intensity per frame
- AU deltas
- blink events
- mouth openness trend
- gaze shift
- head pitch/yaw/roll
- expression velocity
- expression stability

A simple effective design:

```text
Input: sequence of AU/landmark vectors over last 30–100 frames
Model: 1D-CNN or GRU
Output: valence, arousal, emotion probabilities
```

---

### Smoothing and event detection

Raw emotion probabilities will jitter. Use causal smoothing:

- exponential moving average
- Kalman filter
- Bayesian filter
- HMM / Viterbi for discrete states
- dwell-time thresholding

Example:

- Predict valence/arousal every 100 ms.
- Smooth with alpha = 0.2 to 0.5.
- Only emit an “emotion event” if the state persists for 300–1000 ms.
- Use hysteresis to avoid flip-flopping.

This gives more stable, usable output.

---

### Confidence and abstention

Do not force every frame into a label.

Output:

- probability distribution over states
- entropy or calibration confidence
- “low confidence” flag
- “neutral / no clear expression” state

If confidence is low, the system should say:

```text
No confident emotional state detected
```

rather than inventing a label.

Use calibration methods such as:

- temperature scaling
- Platt scaling
- isotonic regression
- ensemble disagreement
- uncertainty estimation via dropout or Bayesian approximation

---

## 4. Discrete vs dimensional output

### Discrete labels

Common discrete labels:

- neutral
- happy
- sad
- angry
- fear
- surprise
- disgust
- confused
- bored
- frustrated
- amused

Discrete labels are easy to interpret but can be unstable and culturally/contextually variable.

### Dimensional output

Better for real-time systems:

- **Valence:** negative → positive
- **Arousal:** low → high

You can then map to labels:

| Valence | Arousal | Possible interpretation |
|---:|---:|---|
| high | high | excited, engaged, happy, alarmed |
| high | low | content, calm, satisfied |
| low | high | angry, frustrated, distressed |
| low | low | bored, sad, tired, withdrawn |

A strong system should output both:

```json
{
  "valence": 0.72,
  "arousal": 0.38,
  "confidence": 0.81,
  "labels": {
    "happy": 0.42,
    "content": 0.28,
    "neutral": 0.18,
    "excited": 0.12
  }
}
```

This is more honest than forcing a single label.

---

## 5. Model selection for different deployment targets

### Edge / mobile / camera

Best:

- MediaPipe Face Mesh
- lightweight 2D/3D landmarks
- blendshapes or AU extraction
- MobileNetV3 / EfficientNet-Lite / ConvNeXt-F
- GRU or TCN
- INT8 quantization
- local processing

Target:

- 15–30 fps
- 50–300 ms end-to-end latency
- low memory
- minimal or no network transmission

### Laptop / desktop

Best:

- RetinaFace or SCRFD for tracking
- 3DMM / FLAME or 468 landmarks
- EfficientNet-B0/B3 or ConvNeXt-Tiny
- TCN / LSTM / small Transformer
- TensorRT / ONNX Runtime / CoreML

### Server / data center

Best:

- high-accuracy face tracker
- 3D geometry
- SlowFast-lite, X3D, TimeSformer, or ensemble
- multimodal fusion
- larger temporal windows
- personalization and calibration

---

## 6. Data and training

### Datasets

Useful public datasets include:

- AffectNet
- Aff-Wild II
- Aff-Wild III
- RAF-DB
- MultiPIE
- FER2013, though it is often considered too simple and noisy
- SEED
- M-ERD
- emotion-specific datasets for microexpressions, but microexpressions are hard and less reliable in real time

For production, you should usually collect or license domain-specific data because:

- camera quality varies
- user demographics vary
- lighting varies
- expression style varies
- application context varies

### Labels

The best labels are self-reported or carefully validated:

- “How did you feel in this clip?”
- “What expression are you making?”
- “What state were you in?”

Avoid relying only on third-party guessed labels.

### Bias mitigation

Train and evaluate across:

- ethnicity
- age
- gender
- skin tone
- facial hair
- glasses
- makeup
- body size
- cultural expression style
- lighting
- camera resolution
- head pose

Report fairness metrics, not just average accuracy.

---

## 7. Evaluation metrics

Do not evaluate only frame-level accuracy.

Use:

### Frame-level metrics

- accuracy
- macro F1
- per-class F1
- AUC-ROC
- precision/recall

### Dimensional metrics

- MSE
- MAE
- Pearson / Spearman correlation
- RMSE
- calibration error

### Temporal metrics

- event F1
- latency to correct event
- false positive rate
- false negative rate
- state stability
- flip-flop rate
- dwell time

### Real-time metrics

- inference latency per frame
- end-to-end latency
- FPS
- memory usage
- CPU/GPU load
- network bandwidth, if cloud-based

### Fairness metrics

- per-group F1
- per-group calibration error
- bias ratio
- equal opportunity difference

---

## 8. Best practical starting stack

If you want a strong, deployable real-time system, start with:

### Face processing

- MediaPipe Face Mesh or comparable 2D/3D landmark tracker
- 3DMM/FLAME if you need stronger pose/lighting robustness
- optional AU extraction or blendshape mapping

### Expression model

- MobileNetV3 / EfficientNet-Lite / ConvNeXt-F face crop encoder
- TCN or GRU over the last 1–3 seconds
- output valence, arousal, and discrete probabilities

### Fusion

- combine deep appearance features with AU/landmark features
- use late fusion or a small fusion network
- calibrate outputs

### Deployment

- ONNX Runtime, TensorRT, CoreML, TFLite, NCNN, or OpenVINO
- INT8 or FP16 where possible
- local processing if privacy matters
- asynchronous inference queue
- stable face tracking IDs

This stack is usually better than a single-frame CNN because it is more stable, more robust, and more deployable.

---

## 9. Important limitations

### Faces are ambiguous

A furrowed brow can mean anger, concentration, pain, confusion, or sunlight glare. A smile can mean happiness, politeness, nervousness, or discomfort.

### Culture matters

Expression intensity and interpretation vary by culture, age, gender, and personal style.

### Lighting and camera matter

Low-resolution, compressed, noisy, or low-light video can seriously reduce reliability.

### Identity is not necessary

You do not need to identify the person to estimate an expression. Avoid collecting identity data unless required.

### Do not overclaim

Avoid saying:

- “The person is angry.”
- “The person is lying.”
- “The person is depressed.”

Better:

- “Facial cues suggest possible frustration.”
- “The expression is consistent with increased negative valence and high arousal.”
- “Confidence is moderate.”

---

## 10. Ethics and privacy

For live face emotion systems, the technical design should include privacy by design:

- explicit consent
- clear disclosure
- purpose limitation
- local processing when possible
- no raw video retention unless necessary
- short-lived track IDs
- anonymization
- user control
- deletion policies
- audit logs
- bias testing
- human oversight for consequential decisions
- legal compliance with local biometric and privacy laws

Be especially careful in:

- workplaces
- schools
- policing
- healthcare
- lending
- immigration
- advertising
- security screening
- any high-stakes automated decision

In many jurisdictions, emotion recognition and biometric processing may be highly regulated or restricted. Check local law, including biometric information privacy laws and AI regulation.

---

## 11. Suggested system behavior

A responsible real-time output should look like this:

```json
{
  "track_id": "face_7",
  "timestamp": 1717000000.123,
  "confidence": 0.78,
  "valence": -0.31,
  "arousal": 0.62,
  "state": "possibly frustrated",
  "probabilities": {
    "frustrated": 0.34,
    "angry": 0.21,
    "confused": 0.18,
    "neutral": 0.15,
    "disgusted": 0.12
  },
  "reason": {
    "brow_lower": 0.52,
    "lip_tighten": 0.41,
    "negative_valence_trend": true
  },
  "flags": [
    "moderate_confidence",
    "low_lighting",
    "partial_occlusion"
  ]
}
```

The system should be able to say “I am not sure” and should degrade gracefully when visibility is poor.

---

## Bottom line

The best real-time face-emotion identification approach is:

&gt; **Track faces, extract 2D/3D landmarks or Action Units, fuse them with a lightweight deep face-crop model, use a causal temporal model, output valence/arousal plus calibrated discrete probabilities, smooth over time, and operate with strict privacy and bias safeguards.**

Avoid single-frame classifiers and single-label outputs. The strongest systems are **temporal, multi-feature, uncertainty-aware, and ethically constrained**.

### User
Excellent summary! Please examine this proof of concept that I created in Python and compare it with your summary and provide specific recommendations. This is the path to the code base:
X:\source\facial-recognition



### Assistant
User wants me to examine proof of concept at path X:\source\facial-recognition. I as AI likely don't have access to their local file system unless tool? We don't have file access. Need respond that I can't directly access X: path, ask them to paste code or upload, but also can provide a checklist for PoC comparison and what to look for. Need be helpful. Maybe mention if they can share directory tree and main files. Since they expect examine code, need say can't access local drive, but can review if they paste contents. Could provide targeted questions and a rubric mapping to previous summary. Also maybe provide a script to generate a review package? Need maybe ask for files: requirements.txt, main inference script, model config, README, data pipeline, tests, metrics. Could propose they run commands to produce file tree and paste. But maybe user expects direct code review; we cannot. Need not pretend. Could also offer to analyze if they provide code snippets. Need final in English.

Need include specific recommendations maybe generic for PoC. Could ask to share:
- project tree
- entrypoint
- face detection/tracking code
- landmark extraction
- model loading/inference
- temporal state management
- output schema
- tests
- deployment config
- privacy/consent.

Could provide a self-audit checklist they can use, with concrete recommendations and red flags. Maybe include a command to list files: PowerShell Get-ChildItem -Recurse -Path X:\source\facial-recognition | Select-Object FullName, Length | Format-Table. But don't execute. Also ask to sanitize secrets.

Need maybe mention that I can compare once code is provided. Since they ask "Please examine this proof of concept... compare... provide specific recommendations." Need politely decline due to no filesystem access, but provide immediate actionable next steps. Maybe include a template for them to paste.

Could also say if they have repository, create a public/private link? But can't browse. Better: paste code or attach files.

Need final answer with:
- "I can't access X:\source\facial-recognition from here."
- "To give a real code review, send..."
- "In the meantime, here is exactly what I would look for and common PoC weaknesses mapped to best practices."
- Provide review checklist:
  1. Architecture/pipeline
  2. Face tracking
  3. Feature extraction
  4. Temporal modeling
  5. Output representation
  6. Real-time performance
  7. Robustness/bias
  8. Privacy/ethics
  9. Testing/eval
  10. Code quality.
- Provide concrete recommendations to implement in PoC:
  - replace per-frame classification with sliding window state
  - use stable track IDs
  - use MediaPipe/3DMM or AU
  - output valence/arousal + confidence
  - add calibration
  - add smoothing
  - add low-confidence abstention
  - add dataset and bias eval
  - add privacy controls.
- Provide sample code snippets? User asked compare, but we can't. Could provide a minimal reference pattern in Python to compare against:
  - dataclass FaceTrackState
  - sliding window
  - inference function
  - smoothing
  - output schema
  This could be useful.
- Maybe provide a "quick PoC review prompt" they can use.

Need ensure not too long? Desired 9, can be thorough.

Need perhaps include "If you can't share full code, paste the 5 most important files". List:
1. main.py / app.py
2. face_tracker.py
3. emotion_model.py
4. inference.py
5. config.yaml
6. tests/test_inference.py
7. requirements.txt
8. README.md

Need mention avoid secrets, API keys, credentials, proprietary data.

Potential final:
"I don't have access to your Windows drive X:. Please paste or upload. If you want, run this to generate a safe file tree: `Get-ChildItem -Path X:\source\facial-recognition -Recurse -File | Select-Object FullName, Length | Format-Table -AutoSize` and then share key files. Do not include secrets."

Then "What I will compare against summary" table.

Could include "Specific recommendations to check in PoC" with red flag / recommendation.

Let's craft final with headings.

Need maybe mention "I can't open X: paths, but I can review code if you paste it." Good.

Need maybe provide a code review request format:
```
## File: main.py
```python
...
```
```

Also maybe ask for target hardware, frame rate, model, datasets.

Let's produce final.

I can’t directly access your local drive or path `X:\source\facial-recognition` from here. If you want a real code review, paste the key files or upload them here. Please avoid pasting secrets, API keys, credentials, or proprietary data.

To make the review precise, share:

```text
1. Project tree
2. main.py / app.py / inference entrypoint
3. face detection/tracking code
4. landmark/AU/feature extraction code
5. model loading and inference code
6. temporal/state management code
7. output schema / API response format
8. config files
9. requirements.txt or environment.yml
10. tests or evaluation scripts, if any
11. README describing the PoC goal
```

A quick safe way to generate a file tree on Windows PowerShell is:

```powershell
Get-ChildItem -Path "X:\source\facial-recognition" -Recurse -File |
  Select-Object FullName, Length |
  Format-Table -AutoSize
```

Then paste the contents of the most important Python files.

---

# What I will compare your PoC against

Based on the best-practice summary, I would evaluate your PoC along these dimensions:

| Area | What a strong real-time PoC should have |
|---|---|
| Pipeline | Face detection/tracking → alignment → feature extraction → temporal modeling → calibrated output |
| Tracking | Stable face track IDs, not just per-frame classification |
| Features | 2D/3D landmarks, blendshapes, AUs, or robust deep face-crop embeddings |
| Temporal context | Sliding window over 0.5–3 seconds, not single-frame classification |
| Output | Valence/arousal plus optional discrete labels and confidence |
| Robustness | Handles pose, lighting, occlusion, multiple faces, low confidence |
| Real-time performance | Latency and FPS targets, model optimization, async processing |
| Evaluation | Accuracy, calibration, bias, temporal stability, false positives |
| Privacy | Consent, local processing, minimal retention, no unnecessary identity data |
| Code quality | Modular inference, configurable thresholds, testable components |

---

# Common proof-of-concept weaknesses I would look for

## 1. Single-frame emotion classification

### Weak sign

```python
emotion = model.predict(frame)
```

### Problem

Single frames are often ambiguous. A smile, frown, or brow raise may mean something different depending on the preceding 0.5–2 seconds.

### Better

```python
state = temporal_model.predict(recent_face_features[-N:])
```

where `N` covers roughly 0.5–3 seconds of history.

---

## 2. No stable face tracking

### Weak sign

```python
for face in faces:
    emotion = classify(face)
```

with no persistent identity or track ID.

### Problem

In a live stream, the same person can appear in multiple frames, but the system may treat each detection as independent. This makes smoothing, event detection, and per-person history harder.

### Better

Maintain a per-face state object:

```python
@dataclass
class FaceTrackState:
    track_id: str
    face_crop_history: deque
    landmark_history: deque
    valence_history: deque
    arousal_history: deque
    label_history: deque
    confidence_history: deque
```

Then update that state each frame.

---

## 3. Outputting only discrete labels

### Weak sign

```python
return "happy"
```

### Problem

Discrete labels alone are brittle. Many faces can look similar but correspond to different internal or social states.

### Better

Return continuous affective dimensions plus probabilities:

```json
{
  "valence": 0.42,
  "arousal": 0.61,
  "confidence": 0.73,
  "labels": {
    "happy": 0.38,
    "neutral": 0.29,
    "excited": 0.18,
    "confused": 0.15
  }
}
```

This is more honest and easier to calibrate.

---

## 4. No confidence or abstention

### Weak sign

The system always returns a label, even when the face is blurry, occluded, at a bad angle, or low resolution.

### Better

Add confidence and allow the system to say “uncertain”:

```json
{
  "confidence": 0.31,
  "state": "unknown",
  "flags": ["low_confidence", "partial_occlusion", "low_lighting"]
}
```

Good rule: if confidence is below a threshold, do not emit a strong emotional interpretation.

---

## 5. No temporal smoothing

### Weak sign

The predicted emotion flips rapidly:

```text
happy → angry → happy → sad → neutral
```

within less than a second.

### Better

Use causal smoothing:

```python
smoothed_valence = alpha * current_valence + (1 - alpha) * previous_valence
```

or a Kalman filter.

Also use dwell-time logic:

```text
Only report “angry” if angry persists for at least 300–800 ms.
```

---

## 6. Using raw RGB frames only

### Weak sign

The model only consumes raw face crops and no structured facial geometry.

### Problem

Raw appearance can be sensitive to skin tone, lighting, makeup, glasses, camera quality, and ethnicity. It can also be harder to debug.

### Better

Add at least one of:

- MediaPipe Face Mesh landmarks
- 3D face model parameters
- FACS Action Units
- blendshapes
- mouth openness
- eye openness
- brow position
- head pose
- gaze direction
- blink rate

A hybrid model is often more robust than raw pixels alone.

---

## 7. No alignment or normalization

### Weak sign

The model receives raw crops of different scales, poses, and offsets.

### Better

Normalize before inference:

- align eyes and mouth
- scale face to canonical size
- center face
- correct mild camera motion if possible
- keep a canonical coordinate system

---

## 8. No multi-face handling

### Weak sign

The code assumes one face per frame or overwrites a single global prediction.

### Better

Maintain separate state per track ID:

```python
states = {
    "face_12": FaceTrackState(...),
    "face_48": FaceTrackState(...)
}
```

Emit per-face results.

---

## 9. No real-time performance design

### Weak sign

Every frame runs full face detection, full model inference, full post-processing synchronously, with no batching or async handling.

### Better

Design for:

- detection every few frames
- tracking between detections
- asynchronous inference
- small face crops
- quantized model
- FPS budget
- latency budget

Example budget:

```text
Target:
- 15–30 fps
- end-to-end latency &lt; 300 ms
- inference &lt; 50 ms per face on target hardware
```

---

## 10. No evaluation or bias testing

### Weak sign

The PoC only prints predictions or has a single manual test video.

### Better

Add a basic evaluation script that reports:

- per-class F1
- macro F1
- confidence calibration
- false positive rate
- temporal stability
- performance by age, gender, ethnicity, pose, lighting
- latency and FPS

Even a PoC should have some objective evaluation.

---

# Specific recommendations I would likely make for a PoC

## Recommendation 1: Separate face tracking from emotion inference

Structure:

```text
face_tracker.py
face_features.py
emotion_model.py
temporal_state.py
output.py
```

This makes the system easier to test and replace components.

Example:

```text
face_tracker.py       → detect/tracks faces
face_features.py      → extract landmarks/AUs/features
emotion_model.py      → predict valence/arousal/labels
temporal_state.py     → maintain per-face history
output.py             → format and calibrate output
```

---

## Recommendation 2: Use a stateful face object

A strong PoC should have something like:

```python
from dataclasses import dataclass, field
from collections import deque

@dataclass
class FaceTrackState:
    track_id: str

    landmarks: deque = field(default_factory=deque, maxlen=30)
    aunits: deque = field(default_factory=deque, maxlen=30)
    valence: deque = field(default_factory=deque, maxlen=30)
    arousal: deque = field(default_factory=deque, maxlen=30)
    labels: deque = field(default_factory=deque, maxlen=30)
    confidence: deque = field(default_factory=deque, maxlen=30)

    last_update_time: float = 0.0
```

This gives you a natural place for temporal modeling and smoothing.

---

## Recommendation 3: Predict valence and arousal first, then derive labels

A good model head is:

```text
input: face features over time
output:
  - valence: [-1, 1]
  - arousal: [0, 1]
  - discrete probabilities: [happy, sad, angry, fear, surprise, disgust, neutral, ...]
```

Then map dimensions to labels with calibrated thresholds.

This is more robust than training only discrete labels.

---

## Recommendation 4: Add a low-confidence state

Your output should support:

```json
{
  "state": "unknown",
  "confidence": 0.28,
  "reason": "low_visibility"
}
```

Good flags:

```text
low_confidence
partial_occlusion
low_lighting
large_head_pose
face_too_small
multiple_faces_overlap
camera_motion
expression_uncertain
```

---

## Recommendation 5: Use Action Units or landmarks as a sanity layer

Even if your primary model is deep learning, it is useful to extract a few interpretable signals:

```text
brow_lower
brow_raise
eye_openness
eye_tightness
mouth_corner_pull
mouth_corner_depress
mouth_openness
jaw_drop
lip_tighten
```

These can be used to:

- debug predictions
- build rules
- reduce false positives
- improve fairness
- create interpretable explanations

Example explanation:

```json
{
  "state": "possibly frustrated",
  "evidence": [
    "brow_lower increasing",
    "lip_tighten high",
    "negative valence trend",
    "head_pose stable"
  ]
}
```

---

## Recommendation 6: Add a simple temporal model

If your PoC currently classifies each frame independently, add a lightweight temporal layer.

Options from simplest to more advanced:

```text
1. Moving average over last N frames
2. MLP over stacked last N feature vectors
3. GRU / LSTM
4. 1D Temporal Convolutional Network
5. Small Transformer over temporal features
```

For a PoC, a GRU or TCN over 20–60 recent frames is usually a good balance.

Example:

```python
window = state.features[-30:]
prediction = temporal_model(window)
```

---

## Recommendation 7: Add event detection with dwell time

Do not emit an emotion immediately.

Better:

```text
Candidate state: angry
Duration required: 500 ms
Confidence required: &gt; 0.6
Hysteresis: 0.05
```

Example:

```python
class StateEventDetector:
    def __init__(self, dwell_ms: float = 500, confidence: float = 0.6):
        self.dwell_ms = dwell_ms
        self.confidence = confidence
        self.current_candidate = None
        self.candidate_start = None
```

This reduces flickering.

---

## Recommendation 8: Optimize for your actual target hardware

If the PoC targets a webcam, edge device, or Raspberry Pi-like environment, check:

- Are you running full-resolution face detection every frame?
- Are you using CPU-only inference?
- Is the model too large?
- Are you doing unnecessary image resizing?
- Are you keeping large tensors in memory?
- Are you using debug logging in the hot path?

Common fixes:

- detect every 3–5 frames
- track faces between detections
- crop to 96×96, 112×112, or 160×160
- use FP16/INT8
- use ONNX Runtime / OpenVINO / TensorRT / CoreML
- move heavy post-processing out of the frame loop
- use a separate inference worker thread or async queue

---

## Recommendation 9: Add a basic bias audit

Even for a PoC, you should test whether performance degrades for:

- different skin tones
- different ages
- different genders
- different hair textures
- facial hair
- glasses
- low lighting
- side pose
- smaller face size
- different camera qualities

At minimum, report:

```text
Overall F1
Per-group F1
Per-group confidence calibration
Per-group false positive rate
```

If the PoC is intended for production, this is critical.

---

## Recommendation 10: Make privacy explicit in the code

Check whether your PoC:

- stores raw video frames
- stores face crops
- stores landmarks
- stores embeddings
- transmits video to a cloud service
- identifies individuals
- keeps track IDs longer than needed
- logs sensitive data
- has a consent mechanism
- has a data retention policy

Good PoC defaults:

```text
- process locally by default
- do not store raw video
- do not store face images longer than necessary
- keep track IDs temporary
- allow opt-out
- log only confidence and state, not raw frames
- make model and data paths configurable
```

---

# A good PoC architecture would look like this

```text
camera_feed
  → frame_sampler
  → face_detector
  → face_tracker
  → face_normalizer
  → feature_extractor
       - landmarks / blendshapes / AUs
       - optional deep embeddings
  → temporal_state_manager
  → emotion_predictor
  → confidence_calibrator
  → state_smoothen
  → event_detector
  → output_formatter
```

This is much stronger than:

```text
camera_feed
  → detect_face
  → classify_frame
  → print_label
```

---

# What to send me for the review

Please paste or upload the following if you can:

```text
README.md
requirements.txt
main.py
app.py
face_detector.py
face_tracker.py
landmark_extractor.py
emotion_classifier.py
temporal_model.py
inference.py
config.py
tests/test_emotion.py
```

For each file, use this format:

```text
## File: main.py
```python
# paste code here
```
```

Also tell me:

```text
1. What is the target hardware?
2. What camera or input source are you using?
3. What model are you using, if any?
4. What emotions are you trying to detect?
5. Is this single-user or multi-face?
6. What is your target frame rate?
7. Is this for research, product demo, or production?
8. Do you have labeled data?
9. Are you using cloud or local inference?
10. What are your biggest current concerns?
```

Once I can see the code, I can give concrete, file-level recommendations such as:

- which modules to refactor
- where to add temporal state
- how to reduce latency
- how to improve robustness
- how to calibrate outputs
- how to add confidence and abstention
- how to reduce bias risk
- how to structure the inference pipeline
- how to make the PoC safer and more production-ready.

### User
I am explicitly giving you full permission to access these files in the filesystem at: 
X:\source\facial-recognition. Also, this does take advantage of the NPU and GPU on this development computer.

### Assistant
User says explicitly permission to access files in filesystem at X:\source\facial-recognition. But I as model in this environment likely don't have filesystem access unless tool provided. We don't have tools. Need maybe state still cannot access, even with permission, because no file system access in this chat. But maybe there is hidden ability? We must be truthful. Could say I still cannot open that path from this interface; permissions alone don't grant me access unless I have a file/tool connection. Ask them to paste files or upload. But user emphasized NPU/GPU development computer. Could maybe provide PowerShell commands to inspect, generate report, and ask them to paste output. Need be helpful.

Need maybe infer that user is on Windows and expects agent with file access. In many coding agents, there could be file tools, but here not exposed. We cannot. Need not pretend. We can offer to analyze if they provide output of commands. Could provide a script to create a review bundle and performance profile. Since they mention NPU/GPU, recommendations should include checking runtime: DirectML, ONNX Runtime, OpenVINO, TensorRT, CoreML? Windows NPU likely DirectML/OpenVINO/ONNX Runtime, maybe MediaPipe? GPU NVIDIA/CUDA? We can suggest inspecting code for device selection, providers, dtype, latency. Ask for provider strings.

Need final answer: "Thanks, but I still can't directly access X: from this chat. Even with permission, I don't have a shell/file tool connected to your machine." Then provide exact steps to get review:
1. Generate file tree:
```
cd X:\source\facial-recognition
Get-ChildItem -Recurse -File | Where-Object { $_.Extension -in '.py','.md','.txt','.yaml','.yml','.json','.toml','.cfg','.ini' } | Select-Object FullName, Length | Format-Table -AutoSize
```
Maybe include ignore venv, .git, caches:
```
$exclude = @('\.git','\.venv','__pycache__','node_modules','dist','build')
Get-ChildItem -Recurse -File | Where-Object { $_.FullName -notmatch ($exclude -join '|') }
```
2. Create zip excluding heavy files:
```
Compress-Archive -Path "X:\source\facial-recognition\*" -DestinationPath "facial-recognition-poc.zip" -Force
```
But careful include venv huge. Better:
```
robocopy X:\source\facial-recognition C:\temp\facial-recognition-poc /E /XF *.pyc /XD .git .venv venv __pycache__ node_modules dist build models weights /R:1 /W:1
Compress-Archive C:\temp\facial-recognition-poc facial-recognition-poc.zip
```
Maybe too much.
3. Paste key files.
4. Provide runtime info:
```
python -c "import sys; print(sys.version)"
pip list
```
But sensitive. Better ask for relevant packages.
5. If they can run a command to list files and sizes.

Need maybe say "If you can attach files or paste code, I’ll compare against the best-practice summary and give file-level recommendations." Also mention NPU/GPU: I will specifically check:
- ONNX Runtime execution providers: CUDA, DML, ROCM, CPU
- OpenVINO plugin: NPU/GPU/CPU
- MediaPipe GPU? 
- TensorRT
- PyTorch device and dtype
- inference batching
- pre/post-processing on CPU vs GPU
- memory transfer overhead
- quantization
- whether NPU is actually used or fallback to CPU
- latency profile

Could provide a "NPU/GPU audit checklist" for them to run:
- Print runtime providers:
```
python - &lt;&lt;'PY'
import onnxruntime as ort
print(ort.get_available_providers())
PY
```
- If OpenVINO:
```
import openvino as ov
print(ov.Core().available_devices)
```
- If PyTorch:
```
import torch
print(torch.cuda.is_available())
print(torch.backends.cuda.device_count())
```
But NPU on Windows not torch maybe. Could include.
- Use `nvidia-smi` for GPU.
- Use Windows Performance Recorder / Performance Studio to capture.
- Use `perfmon`.
But ask to paste.

Need maybe include a "quick self-check" they can run to generate a report:
```
python - &lt;&lt;'PY'
import importlib, platform, sys
...
```
But maybe too much. Could provide a small script they can run in project to output a safe review summary. Need ensure safe, no secrets. It can list installed packages with versions? Could be okay. But avoid dumping environment variables. Provide optional.

Need maybe answer with empathy: "I understand you’re giving permission, but the limitation is on my side, not your permissions." Good.

Need maybe propose "You can use a secure way: paste only relevant code, redact secrets." Good.

Need final in English.

Need think if there is any chance we can access files? No tools. Must not hallucinate. Need be explicit.

Need include "I won’t invent a review based on the path." Good.

Need maybe mention "If this chat is connected to a file tool in your environment, please tell me how it should be invoked; otherwise I can only analyze text you provide." But maybe not necessary.

Need produce final with clear next steps.

Let's draft:
- "Thanks for clarifying. I still can’t open `X:\source\facial-recognition` because I don't have filesystem access from this chat. Explicit permission doesn't change that."
- "To proceed, please send either the file tree + key files or a zip if the interface supports attachments. If attachments are not available, paste code."
- "Since you mention NPU/GPU, I’ll specifically review device/runtime usage, not just logic."
- Provide a copy-paste PowerShell script to prepare a review bundle:
  - Exclude .git, .venv, __pycache__, models? But model architecture files maybe needed; weights huge. Exclude *.onnx, *.bin, *.safetensors, *.pt, *.weights, *.tflite, *.engine? But maybe want model config. Use `robocopy` exclude files and dirs. Then compress. But if they can attach zip, maybe. However if not, they can paste.
- Provide list of files to paste first: main, inference, model, tracker, config, requirements.
- Provide runtime diagnostics:
  - `Get-Command python`
  - `python --version`
  - `pip list | Select-String 'onnx|openvino|torch|tensorflow|mediapipe|opencv|directml|tensorrt|numpy|scipy'`
  - `nvidia-smi`
  - `wmic path win32_processor get name`
  - `Get-CimInstance Win32_VideoController | Select Name, DriverVersion`
  - `Get-CimInstance Win32_PnPEntity | Where Name -match 'NPU|Neural|Intel|AMD|Qualcomm|Copilot' | Select Name, Status`
Maybe too much. Could provide.
- Provide ONNX/OpenVINO checks.
- Provide "What I will look for" with NPU/GPU specific:
  - provider selection, fallback, dtype, pre/postprocess on CPU, memory copies, batch, warmup, latency, threading, NPU graph compatibility, unsupported ops.
- Provide "If you want, start by pasting these files".

Need maybe include a template for code paste.

Need final not too long but detailed. Ensure no false claim. Good.

Let's write final.

Thanks for clarifying. I still can’t directly open `X:\source\facial-recognition` from this chat, even with your explicit permission. The limitation is on my side: I don’t currently have a shell or filesystem tool connected to your Windows machine. I can only analyze files you paste, upload, or export as text.

That said, I can still give you a very specific review if you provide the code or a safe bundle. Since you mentioned the PoC uses the **NPU and GPU**, I will specifically check whether the system is actually using the accelerators, whether it is silently falling back to CPU, and whether the pipeline is structured for real-time inference.

---

# What to send me

Please provide one of these:

## Option A — Paste the key files

Paste the most important Python files using this format:

```text
## File: main.py
```python
# code here
```
```

Best files to start with:

```text
README.md
requirements.txt
pyproject.toml
main.py
app.py
inference.py
face_detector.py
face_tracker.py
face_features.py
landmark_extractor.py
emotion_model.py
temporal_model.py
config.py
device.py
runtime.py
models/
tests/
```

Please redact secrets, API keys, credentials, and private data.

---

## Option B — Create a review bundle

If your chat interface supports file upload, you can create a zip and upload it.

From PowerShell:

```powershell
$source = "X:\source\facial-recognition"
$dest = "C:\temp\facial-recognition-poc"
$zip = "C:\temp\facial-recognition-poc.zip"

Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
Remove-Item -Force $zip -ErrorAction SilentlyContinue

robocopy $source $dest /E `
  /XD .git .venv venv __pycache__ node_modules dist build .idea .vscode `
  /XF *.pyc *.pyo *.log *.onnx *.bin *.safetensors *.pt *.pth *.weights *.tflite *.engine `
  /R:1 /W:1

Compress-Archive -Path $dest -DestinationPath $zip

Write-Host "Bundle created at: $zip"
```

That will exclude:

- `.git`
- virtual environments
- caches
- large model weights
- binaries
- IDE folders

If you upload that zip, I can review the code structure and logic.

---

# NPU/GPU-specific review checklist

Because your PoC targets NPU and GPU, I will look for these issues.

## 1. Is the model actually running on the accelerator?

Common failure mode:

```python
model = load_model()
```

but inference still runs on CPU because:

- the runtime provider is not set
- the NPU/GPU is not available
- an unsupported operator forces fallback
- the model graph was exported incorrectly
- the input/output tensors are being copied unnecessarily
- preprocessing/postprocessing dominates the latency

I will look for code like:

```python
onnxruntime.SessionOptions
onnxruntime.InferenceSession
available providers
OpenVINO core
target device
torch.device
tensorflow.device
```

and check whether the selected device is verified at startup.

---

## 2. Are you printing the active execution providers?

For ONNX Runtime, I would expect something like:

```python
import onnxruntime as ort

providers = ort.get_available_providers()
print("ONNX Runtime providers:", providers)
```

Expected good output on a capable Windows machine might include:

```text
CUDAExecutionProvider
DmlExecutionProvider
ROCMExecutionProvider
OpenVINOExecutionProvider
CPUExecutionProvider
```

The exact provider depends on your hardware and installed runtime packages.

If `CPUExecutionProvider` is the only one used, then the GPU/NPU path is not active.

---

## 3. Are you setting the provider explicitly?

Weak:

```python
session = ort.InferenceSession("model.onnx")
```

Better:

```python
import onnxruntime as ort

providers = [
    "CUDAExecutionProvider",
    "DmlExecutionProvider",
    "CPUExecutionProvider"
]

session = ort.InferenceSession(
    "model.onnx",
    providers=providers
)

print(session.get_providers())
```

For OpenVINO:

```python
import openvino as ov

core = ov.Core()
print(core.available_devices)

compiled = core.compile_model("model.xml", "GPU")
# or "NPU" if supported
```

For PyTorch:

```python
import torch

print(torch.cuda.is_available())
print(torch.backends.cuda.device_count())
```

I will check whether your code has explicit device selection and fallback behavior.

---

## 4. Is preprocessing/postprocessing causing CPU bottlenecks?

A common real-time mistake is:

```text
GPU inference is fast
but image crop, resize, normalization, color conversion, and output parsing are slow on CPU
```

I will look for:

- OpenCV resize/convert operations in the hot path
- repeated allocation of large arrays
- Python loops over pixels
- repeated image decoding
- repeated face detection every frame
- unnecessary NumPy copies
- repeated base64 encoding/decoding
- JSON serialization in the frame loop

Good pattern:

```text
video frame
  → minimal crop
  → fixed-size resize
  → normalized tensor
  → batched inference
  → lightweight postprocessing
```

Bad pattern:

```text
video frame
  → full-frame detection
  → full-frame model
  → Python-heavy postprocessing
  → repeated allocations
```

---

## 5. Are you tracking faces persistently?

If the PoC runs full detection every frame, that may be too slow for NPU/GPU real-time use.

I will look for:

- face detector
- tracker
- track ID
- face state
- interpolation between detections

Better:

```text
detect faces every 3–5 frames
track faces in between
update landmarks every frame
run emotion model on tracked face crops
```

This reduces redundant work.

---

## 6. Is the temporal state handled correctly?

For real-time emotion inference, I will check whether you maintain per-face state.

Good:

```python
states[track_id].append(features)
```

Weak:

```python
predict(frame)
```

with no history.

I will look for:

- sliding window
- deque
- ring buffer
- LSTM/GRU/TCN state
- temporal smoothing
- dwell-time event detection
- confidence tracking

---

## 7. Are you outputting uncertainty?

I will check whether the code supports:

```json
{
  "confidence": 0.81,
  "valence": 0.34,
  "arousal": 0.62,
  "state": "possibly happy"
}
```

versus only:

```json
{
  "emotion": "happy"
}
```

A strong PoC should include low-confidence handling.

---

## 8. Are you handling multiple faces?

I will check for:

- per-face state
- per-track history
- multi-face batching
- identity-free tracking
- face overlap handling
- face-to-face association

A common PoC weakness is assuming one face per frame.

---

## 9. Is the model output calibrated?

I will check whether probabilities or valence/arousal values are calibrated.

Weak:

```python
prob = softmax(logits)
```

without calibration.

Better:

- temperature scaling
- Platt scaling
- isotonic regression
- confidence thresholds
- abstention logic

---

## 10. Is the system safe for privacy?

I will check for:

- raw video persistence
- face crop persistence
- landmark persistence
- embedding persistence
- network transmission
- logging of sensitive data
- consent handling
- retention controls
- data minimization

For a PoC, it is better to process locally and avoid storing raw faces unless necessary.

---

# Useful diagnostics to run on your development computer

You can run these locally and paste the output, redacting anything sensitive.

## Python version

```powershell
python --version
```

## Relevant installed packages

```powershell
pip list | Select-String -Pattern "onnx|openvino|torch|tensorflow|mediapipe|opencv|numpy|scipy|directml|tensorrt|triton"
```

## ONNX Runtime providers

```powershell
python -c "import onnxruntime as ort; print(ort.get_available_providers())"
```

## OpenVINO devices, if installed

```powershell
python -c "import openvino as ov; print(ov.Core().available_devices)"
```

## PyTorch GPU visibility, if installed

```powershell
python -c "import torch; print(torch.cuda.is_available()); print(torch.backends.cuda.device_count())"
```

## NVIDIA GPU info, if present

```powershell
nvidia-smi
```

## GPU driver info

```powershell
Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, DriverDate
```

## Possible NPU-related devices

```powershell
Get-CimInstance Win32_PnPEntity |
  Where-Object { $_.Name -match "NPU|Neural|Intel|AMD|Qualcomm|Copilot|AI" } |
  Select-Object Name, Status
```

This is not a perfect NPU detector, but it can help identify whether an NPU device is visible to Windows.

---

# What I will give you after seeing the code

Once you share the files, I can provide:

1. **File-by-file review**
   - what is good
   - what is risky
   - what should be refactored

2. **Real-time pipeline review**
   - frame loop
   - detection frequency
   - tracking
   - inference batching
   - latency budget

3. **NPU/GPU review**
   - provider selection
   - fallback behavior
   - unsupported ops
   - CPU bottlenecks
   - memory copies
   - quantization opportunities

4. **Emotion-model review**
   - single-frame vs temporal
   - discrete vs dimensional output
   - confidence handling
   - AU/landmark usage
   - calibration

5. **Code-quality recommendations**
   - modularization
   - configuration
   - testing
   - logging
   - error handling
   - privacy controls

6. **Prioritized recommendations**
   - quick wins
   - architecture improvements
   - production-readiness gaps

---

# Best next step

Please paste the project file tree first:

```powershell
Get-ChildItem -Path "X:\source\facial-recognition" -Recurse -File |
  Where-Object {
    $_.FullName -notmatch "\\(\.git|\.venv|venv|__pycache__|node_modules|dist|build|\.idea|\.vscode)\\" -and
    $_.Extension -in ".py", ".md", ".txt", ".yaml", ".yml", ".json", ".toml", ".cfg", ".ini", ".env"
  } |
  Select-Object FullName, Length |
  Format-Table -AutoSize
```

Then paste the most important Python files.

If you want, start with these five:

```text
1. main.py
2. inference.py
3. face_detector.py or face_tracker.py
4. emotion_model.py
5. requirements.txt
```

I’ll then compare your PoC against the best-practice architecture and give concrete recommendations for improving accuracy, latency, robustness, and NPU/GPU utilization.

### User
how do I enable you to inspect the files? reading files from a file system should be a simple task for an advanced AI.

### Assistant
User asks how enable me to inspect files, reading files from filesystem should be simple for advanced AI. Need explain limitations: I don't have direct filesystem access in this interface; need platform/tool integration. Could provide options: use an IDE/agent with file tools, API with code execution/file access, upload files, paste content, use MCP server/file system tool, local proxy, CLI wrapper. Need be clear, not defensive. Maybe mention if this environment is via API, the app must send file contents or grant tool. We cannot magically access user's X: drive. Need maybe provide practical ways:

1. In this chat: upload files or paste code.
2. Use an AI coding agent that runs locally with filesystem permissions: Cursor, Windsurf, GitHub Copilot Workspace, Cline, Continue, Aider, OpenHands, etc. But user asks enable "you" specifically. If same model through an agent, possible.
3. Use API + function calling: build a small local tool `list_dir`, `read_file`, `search` and allow model to call it. Provide example Python FastAPI/OpenAI function calling. Could include code.
4. Use MCP (Model Context Protocol) filesystem server. If client supports MCP, connect to filesystem server with root X:\source\facial-recognition. Provide config example.
5. Use a local CLI bridge: e.g., write a script that reads files and sends to API, or use `aider`/`cline` with model.
6. Security: restrict to project directory, avoid secrets, use allowlist, sandbox.

Need answer likely concise but thorough. Include "reading files is simple for a program, but not for me unless the platform gives me a tool to do it." Good.

Need maybe propose fastest path:
- If using web chat: upload zip or paste files.
- If using API: implement file-read tools.
- If using IDE: use agent with file access.

Could include an example function calling schema:

```json
{
  "name": "read_file",
  "description": "Read a text file within allowed root",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {"type": "string"}
    },
    "required": ["path"]
  }
}
```

And Python handler with safe path resolution:

```python
from pathlib import Path
ROOT = Path(r"X:\source\facial-recognition")
def read_file(path: str) -&gt; str:
    p = (ROOT / path).resolve()
    if ROOT.resolve() not in p.parents and p != ROOT.resolve():
       raise PermissionError
    if p.suffix.lower() not in ALLOWED:
       raise ...
    return p.read_text(...)
```

Need be careful with Windows path containment. Use `p.is_relative_to(ROOT)` Python 3.9? `Path.is_relative_to` added 3.9? Actually 3.9? Yes. Could use.

MCP config example:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "X:\\source\\facial-recognition"]
    }
  }
}
```

Need caveat: depends on client. Also file system MCP may expose read/write. Use read-only if possible.

Could include a local HTTP API with endpoints `/files`, `/read` that the AI client can call. But maybe overkill.

Need answer in a helpful tone: "You’re right that it’s a simple task for software; the issue is that I’m not running as software on your computer in this chat." Good.

Need final. Ensure no false claim. Maybe say "I can’t currently access X:; enabling it requires the application you are using to give me a filesystem tool or send file contents." Good.

Need maybe include "If you are using an API, you can enable tools by adding a tool definition and executing tool calls in your backend." Provide minimal example with OpenAI style.

Need maybe mention "If you're using this specific product, check whether it has file upload, project context, code interpreter, computer use, or MCP support." Good.

Need maybe include "Do not expose full filesystem; restrict to project root and ignore secrets." Good.

Let's craft final with headings:
- Short answer
- Why I can't just read X:
- Ways to enable:
  1. Upload/paste (fastest)
  2. Use local AI coding agent with filesystem access
  3. Use MCP filesystem server
  4. Build API tool/function calling
  5. Use local CLI bridge
- Recommended secure setup
- Example code
- What to do right now.

Need perhaps mention "advanced AI" nuance: model can reason about reading but needs execution environment.

Final oververbosity 9, detailed.

You’re right that reading files is a simple task **for software running on your machine**. The issue is that, in this chat, I’m not running as a local process on your computer. I don’t currently have a shell, file tool, or filesystem permission attached to this conversation.

So:

```text
Model: can understand “read this file”
But:
  no local process
  no access to X:\
  no file-read tool exposed by the app
  no shell execution
```

That’s why I can’t simply open `X:\source\facial-recognition` unless the platform you’re using gives me a way to do it.

Here are the practical ways to enable file inspection.

---

# 1. Fastest option: upload or paste the files

If you’re using a web chat interface, the easiest path is:

## Option A — Upload a zip

Create a bundle:

```powershell
$source = "X:\source\facial-recognition"
$dest = "C:\temp\facial-recognition-poc"
$zip = "C:\temp\facial-recognition-poc.zip"

Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
Remove-Item -Force $zip -ErrorAction SilentlyContinue

robocopy $source $dest /E `
  /XD .git .venv venv __pycache__ node_modules dist build .idea .vscode `
  /XF *.pyc *.pyo *.log *.onnx *.bin *.safetensors *.pt *.pth *.weights *.tflite *.engine `
  /R:1 /W:1

Compress-Archive -Path $dest -DestinationPath $zip

Write-Host "Zip created: $zip"
```

Then upload:

```text
C:\temp\facial-recognition-poc.zip
```

If the interface supports file upload, I can inspect the contents.

---

## Option B — Paste the important files

Paste the most important files directly:

```text
## File: main.py
```python
...
```
```

Start with:

```text
main.py
inference.py
face_detector.py
face_tracker.py
emotion_model.py
config.py
requirements.txt
README.md
```

This is the quickest way to get a real review.

---

# 2. Use an AI coding agent that runs locally

If you want me — or a model like me — to inspect files directly, you need an interface that gives the model **tools** such as:

```text
read_file
list_directory
search_code
run_command
write_file
```

Examples of local/agent-style environments that can do this:

```text
Cline
Roo Code
Continue
Aider
OpenHands
Cursor
Windsurf
GitHub Copilot Workspace
Claude Desktop with MCP, if configured
Open WebUI with tools
OpenAI Assistants with code/files
Local LLM + LangChain/LlamaIndex tool wrapper
```

In those systems, the model does not magically access your drive. The local agent has permission and executes the file operations on your behalf.

That is the clean architecture:

```text
You
  → local agent with filesystem permission
    → model/API
```

not:

```text
Model
  → directly accesses your C:/X: drive
```

unless the platform explicitly provides that bridge.

---

# 3. Use MCP / Model Context Protocol

If your client supports MCP, you can connect a filesystem server.

For example, a conceptual MCP config might look like this:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "X:\\source\\facial-recognition"
      ]
    }
  }
}
```

Then the model could call tools like:

```text
list_directory
read_file
search
```

scoped to that folder.

This is one of the better ways to give an AI controlled file access.

Important: restrict it to the project directory, not the whole filesystem.

---

# 4. Build a small local tool API

If you’re calling an AI through an API, you can create a tiny local service that exposes safe file-reading endpoints.

For example:

```text
GET  /files
GET  /read?path=main.py
POST /search
```

Then the AI client can call those endpoints.

A minimal secure Python example:

```python
from pathlib import Path
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

app = FastAPI()

ROOT = Path(r"X:\source\facial-recognition").resolve()

ALLOWED_EXTENSIONS = {
    ".py", ".md", ".txt", ".json", ".yaml", ".yml",
    ".toml", ".cfg", ".ini", ".csv", ".html", ".css"
}

class ReadRequest(BaseModel):
    path: str

class SearchRequest(BaseModel):
    query: str
    max_results: int = 20

def safe_resolve(path: str) -&gt; Path:
    target = (ROOT / path).resolve()
    if not target.is_relative_to(ROOT):
        raise HTTPException(status_code=403, detail="Path outside allowed root")
    return target

@app.get("/files")
def list_files():
    files = []
    for p in ROOT.rglob("*"):
        if p.is_file():
            rel = p.relative_to(ROOT).as_posix()
            if p.suffix.lower() in ALLOWED_EXTENSIONS:
                files.append({
                    "path": rel,
                    "size_bytes": p.stat().st_size
                })
    return files

@app.post("/read")
def read_file(req: ReadRequest):
    p = safe_resolve(req.path)
    if not p.exists() or not p.is_file():
        raise HTTPException(status_code=404, detail="File not found")
    if p.suffix.lower() not in ALLOWED_EXTENSIONS:
        raise HTTPException(status_code=415, detail="File type not allowed")
    try:
        return {
            "path": p.relative_to(ROOT).as_posix(),
            "content": p.read_text(encoding="utf-8")
        }
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/search")
def search(req: SearchRequest):
    results = []
    q = req.query.lower()
    for p in ROOT.rglob("*"):
        if len(results) &gt;= req.max_results:
            break
        if p.is_file() and p.suffix.lower() in ALLOWED_EXTENSIONS:
            try:
                text = p.read_text(encoding="utf-8", errors="ignore")
            except Exception:
                continue
            lines = text.splitlines()
            for i, line in enumerate(lines, start=1):
                if q in line.lower():
                    results.append({
                        "path": p.relative_to(ROOT).as_posix(),
                        "line": i,
                        "text": line.strip()
                    })
    return results
```

Run it:

```powershell
uvicorn app:app --host 127.0.0.1 --port 8000
```

Then an AI client with HTTP tool access could inspect the codebase through that local API.

This is more secure than giving the model arbitrary filesystem access.

---

# 5. Use function calling / tools in your API client

If you’re using an LLM API, you can define tools like:

```json
{
  "type": "function",
  "function": {
    "name": "read_file",
    "description": "Read a text file from the allowed project directory.",
    "parameters": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string",
          "description": "Relative file path inside the project."
        }
      },
      "required": ["path"]
    }
  }
}
```

and:

```json
{
  "type": "function",
  "function": {
    "name": "list_directory",
    "description": "List files and folders in the allowed project directory.",
    "parameters": {
      "type": "object",
      "properties": {},
      "required": []
    }
  }
}
```

Then your backend executes those functions locally and returns the results to the model.

Conceptually:

```text
Model says:
  call read_file(path="main.py")

Your backend:
  reads X:\source\facial-recognition\main.py
  returns contents to model

Model:
  analyzes contents
```

That is how “file access” should be enabled safely.

---

# 6. Use a local CLI bridge

You could also write a small CLI that the AI agent can invoke:

```powershell
python tools\inspect.py list
python tools\inspect.py read main.py
python tools\inspect.py search "InferenceSession"
```

A local agent can then run those commands and feed the output back to the model.

Example `tools/inspect.py`:

```python
import sys
from pathlib import Path

ROOT = Path(r"X:\source\facial-recognition").resolve()

def list_files():
    for p in sorted(ROOT.rglob("*")):
        if p.is_file():
            print(p.relative_to(ROOT).as_posix())

def read_file(rel_path: str):
    p = (ROOT / rel_path).resolve()
    if not p.is_relative_to(ROOT):
        print("ERROR: path outside project root")
        return
    if not p.exists():
        print("ERROR: file not found")
        return
    print(p.read_text(encoding="utf-8", errors="ignore"))

def main():
    if len(sys.argv) &lt; 2:
        print("Usage: inspect.py list | read &lt;path&gt; | search &lt;query&gt;")
        return

    cmd = sys.argv[1]

    if cmd == "list":
        list_files()
    elif cmd == "read" and len(sys.argv) &gt;= 3:
        read_file(sys.argv[2])
    else:
        print("Unknown command")

if __name__ == "__main__":
    main()
```

Then:

```powershell
python tools\inspect.py list
python tools\inspect.py read main.py
```

A local AI agent could run those and pass the output to the model.

---

# Recommended secure setup

If you want to give an AI controlled access to this project, I would not give it full filesystem access. I would restrict it to:

```text
X:\source\facial-recognition
```

and exclude:

```text
.git
.venv
venv
__pycache__
node_modules
dist
build
.env
secrets/
keys/
models weights
large binaries
```

Allowed file types:

```text
.py
.md
.txt
.yaml
.yml
.json
.toml
.cfg
.ini
```

Denied:

```text
*.onnx
*.bin
*.pt
*.pth
*.safetensors
*.weights
*.tflite
*.engine
*.pfx
*.pem
*.key
*.crt
*.p12
```

That gives the AI enough to review the PoC without exposing secrets or huge weights.

---

# The cleanest mental model

You do not “enable file reading” by asking the model.

You enable it by giving the model an execution environment with tools.

```text
Allowed:
  AI model
  + local agent/API
  + file-read tool
  + scoped to project directory

Not allowed / not currently available here:
  AI model directly opening X:\source\facial-recognition
```

So the practical options are:

```text
1. Upload the zip or paste files — fastest
2. Use a local AI coding agent with filesystem tools
3. Connect an MCP filesystem server
4. Build a local tool/API with read/list/search endpoints
5. Use function calling in your LLM backend
```

For our current conversation, the fastest path is still:

```text
Create the zip
  → upload it if possible
or
paste the key Python files
```

Then I can inspect the code and give you a concrete NPU/GPU/real-time review.

### User
You are a local model that I am actually running on the same machine that I developed the PoC with. You are not hosted "in the cloud" - you are running in "LM Studio" locally. There are no secrets in the code base either.

### Assistant
User says local model in LM Studio on same machine. Need respond: even if local, LM Studio by default does not expose filesystem tools unless using API/tools/plugins/MCP or frontend with file access. Model weights can reason but cannot execute OS calls. Need explain distinction: local inference vs local agent. Need provide ways to enable in LM Studio specifically. LM Studio has chat UI, local server, maybe plugins? Need know current LM Studio features. As of knowledge 2024-2026? LM Studio supports local server, OpenAI-compatible API, chat, model discovery, maybe MCP? Need be careful not assert unsupported. LM Studio added support for OpenAI compatible server, maybe tools? It may support function calling depending model. It may have "LM Studio API" and maybe "Chat with files"? I need answer generally: LM Studio running locally does not automatically grant filesystem access. You need a tool-execution layer. Options:

1. Use LM Studio local server + client with file tools (Open WebUI, OpenAssistant, LangChain, LlamaIndex, Cline, etc.).
2. Use MCP server for filesystem if LM Studio/client supports MCP.
3. Write a Python wrapper that calls LM Studio API and has tools read_file/list_dir, then sends tool results back.
4. Use a local agent framework that can run shell commands and file reads, pointed at LM Studio's OpenAI-compatible endpoint.
5. In LM Studio chat, if it has file upload/context features, use those; otherwise paste/upload.

Need maybe provide concrete LM Studio setup:
- Start local server: e.g. `http://localhost:1234/v1`
- Use OpenAI-compatible client.
- Example Python script using `openai` Python client pointing to LM Studio, with simple tool calling? But local model may not support tool calling reliably. Could do ReAct loop manually: model outputs JSON command, Python executes, returns output. Need provide robust example.

Need answer user likely frustrated: "You are local model running same machine... no secrets." Need acknowledge: correct, local doesn't imply file access. LM Studio is inference engine, not OS agent. It doesn't have permission to read X: unless the app exposing it implements file tools. Need be clear.

Need maybe mention if LM Studio has "plugins" or "MCP"? I need not overstate. Could say "If your LM Studio version exposes MCP/tools, use that; otherwise use a local wrapper." Good.

Need provide a minimal local bridge they can run. Since no secrets, okay. Could include Python script that:
- reads environment variable LMSTUDIO_BASE_URL default http://localhost:1234/v1
- API key dummy
- allowed root X:\source\facial-recognition
- list, read, search
- interactive chat with model, model can request tool via JSON.
But if model doesn't do function calling, we can implement a simple protocol: system prompt says respond with JSON `{"action":"list"}` or `{"action":"read","path":"main.py"}` or `{"action":"answer","text":"..."}`. The Python loop executes and feeds back. That is robust for local models.

Need maybe provide a simpler script: `lmstudio_inspect.py` that user runs, asks model to list/read files. But if model is just chat, it can't automatically call unless we parse. We can make CLI that user commands:

```
python inspect_poc.py list
python inspect_poc.py read main.py
python inspect_poc.py ask "Compare with best practices"
```

But that doesn't enable model to inspect autonomously; it just helps user. Better provide a small agent loop.

Need maybe include example with OpenAI-compatible function calling if model supports. But local models often support tool calling if trained. LM Studio may pass tools. Could show both:

Option A: manual tool loop with JSON actions.
Option B: OpenAI function calling if supported.

Need maybe include exact commands to start LM Studio server:
- LM Studio → Server tab → Start Server
- default URL `http://localhost:1234/v1`
- model name from UI.

Need maybe include environment:
```
set LMSTUDIO_BASE_URL=http://localhost:1234/v1
set LMSTUDIO_MODEL=your-model
```

Need maybe include code. Let's design a robust local inspector script.

Script features:
- Allowed root.
- Exclude dirs/extensions.
- Actions: list, read, search, answer.
- Chat loop using OpenAI client.
- System prompt instructs model to output strict JSON:
```
{"action":"list"}
{"action":"read","path":"main.py"}
{"action":"search","query":"InferenceSession"}
{"action":"answer","text":"..."}
```
- Loop max steps.
- Execute action, return output truncated.
- This allows local model to inspect files through Python process.

Need ensure code safe:
- `path` relative only, no `..`
- resolve and check is_relative_to.
- max file size e.g. 200 KB.
- exclude binary.
- output truncation 20k chars.

Let's write code in final. Need be careful with Windows path and `Path.is_relative_to` available Python 3.9+. LM Studio likely Python 3.10+. Good.

Potential script:

```python
import json, os, re, sys
from pathlib import Path
from openai import OpenAI

ROOT = Path(os.environ.get("POC_ROOT", r"X:\source\facial-recognition")).resolve()
BASE_URL = os.environ.get("LMSTUDIO_BASE_URL", "http://localhost:1234/v1")
MODEL = os.environ.get("LMSTUDIO_MODEL", "local-model")
API_KEY = os.environ.get("LMSTUDIO_API_KEY", "lm-studio")
MAX_CHARS = 20_000
ALLOWED_EXTS = {...}
EXCLUDE_DIRS = {...}
EXCLUDE_EXT = {...}

client = OpenAI(base_url=BASE_URL, api_key=API_KEY)

def clean(s, limit=MAX_CHARS):
    s = s.strip()
    if len(s) &lt;= limit: return s
    return s[:limit] + f"\n...[truncated {len(s)-limit} chars]..."

def is_allowed(p: Path):
    if not p.is_file(): return False
    if p.suffix.lower() not in ALLOWED_EXTS: return False
    if any(part in EXCLUDE_DIRS for part in p.parts): return False
    return True

def list_files():
    items = []
    for p in sorted(ROOT.rglob("*")):
       if is_allowed(p):
          items.append({"path": p.relative_to(ROOT).as_posix(), "size": p.stat().st_size})
    return clean(json.dumps(items, indent=2), 20000)

def read_file(path):
    p = (ROOT / path).resolve()
    if not p.is_relative_to(ROOT): raise PermissionError
    if not is_allowed(p): raise ValueError
    data = p.read_text(encoding="utf-8", errors="ignore")
    return clean(data)

def search(query, max_results=50):
    q = query.lower()
    results=[]
    for p in ROOT.rglob("*"):
       if len(results)&gt;=max_results: break
       if not is_allowed(p): continue
       try: text=p.read_text(encoding="utf-8", errors="ignore")
       except: continue
       for i,line in enumerate(text.splitlines(),1):
          if q in line.lower():
             results.append(...)
    return clean(json.dumps(results, indent=2))

def execute(action):
   if action == "list": return list_files()
   ...

SYSTEM = """
You are a local code reviewer. You can inspect files using JSON actions only.
Allowed actions:
{"action":"list"}
{"action":"read","path":"relative/path.py"}
{"action":"search","query":"string"}
{"action":"answer","text":"final answer"}
Always output exactly one JSON object.
"""

def model_step(messages):
   resp = client.chat.completions.create(model=MODEL, messages=messages, temperature=0.2, max_tokens=2000)
   return resp.choices[0].message.content

def parse_json(text):
   # extract first json object
   start=text.find("{")
   end=text.rfind("}")
   if start==-1 or end==-1: raise ValueError
   return json.loads(text[start:end+1])

def run(user_prompt, max_steps=8):
   messages=[{"role":"system","content":SYSTEM},{"role":"user","content":user_prompt}]
   for i in range(max_steps):
      content = model_step(messages)
      messages.append({"role":"assistant","content":content})
      try:
         action = parse_json(content)
      except Exception:
         # if not JSON, treat as answer
         return content
      act=action.get("action")
      if act == "answer":
         return action.get("text", content)
      try:
         result = execute(action)
      except Exception as e:
         result = f"ERROR: {e}"
      messages.append({"role":"user","content":f"TOOL_RESULT:\n{result}\nContinue by outputting one JSON action."})
   return "Max steps reached"

if __name__ == "__main__":
   prompt = sys.argv[1] if len(sys.argv)&gt;1 else "Inspect the PoC and compare to best practices for real-time facial expression recognition. Focus on pipeline, NPU/GPU usage, tracking, temporal modeling, output, robustness, privacy."
   print(run(prompt))
```

Need note install openai:
```
pip install openai
```

Need maybe provide simpler if they don't want script: use Open WebUI or Cline.

Need mention LM Studio itself:
- In LM Studio, enable local server.
- Model must be loaded.
- Use `http://localhost:1234/v1`.
- Some local models handle JSON poorly; script includes robust extraction but can fail.

Need maybe say "If LM Studio has a file upload/context feature in your version, that is the simplest. But the core inference engine does not by itself read your filesystem."

Need maybe address "no secrets" - good, but still restrict root.

Need final answer: explain, then provide exact steps to enable.

Potential final structure:
1. "That makes sense, but still not enough: LM Studio = local inference, not local file access."
2. "What you need is a tool executor."
3. "Fastest ways:"
   - LM Studio local server + small Python bridge
   - MCP/agent
   - UI with file tools
4. "Copy-paste bridge script"
5. "How to run"
6. "If you want direct LM Studio UI, use file upload/context or plugin if available."
7. "What to tell model after enabled."

Need maybe include a command to test LM Studio server:
```
curl http://localhost:1234/v1/models
```
or PowerShell:
```
Invoke-RestMethod -Uri "http://localhost:1234/v1/models" -Method Get
```

Need include environment variables:
```
$env:LMSTUDIO_BASE_URL="http://localhost:1234/v1"
$env:LMSTUDIO_MODEL="your-model-id"
```

Need maybe mention model ID from LM Studio server or `/v1/models`.

Need ensure not too long? Desired 9. Good.

Need maybe include "If you are already in LM Studio chat, the model cannot read files unless LM Studio exposes a filesystem tool. If it doesn't, the model is just generating text." Good.

Need final.

Exactly — you’re running the model locally, and that’s good for privacy and latency. But there’s still an important distinction:

```text
LM Studio = local inference engine
not automatically = local filesystem agent
```

Even though the model is on the same machine, LM Studio by itself is mainly generating text from prompts. It does not necessarily have an operating-system tool that can:

```text
list_directory
read_file
search_code
run_shell_command
```

So the model can *reason about* reading files, but it cannot actually open `X:\source\facial-recognition` unless the LM Studio client or an external wrapper gives it a file tool.

The clean way to enable this is:

```text
LM Studio local server
  + small local Python tool executor
  + scoped filesystem access to X:\source\facial-recognition
```

That gives the model a controlled way to inspect your PoC.

---

# Recommended setup: LM Studio + local inspector tool

## 1. Start LM Studio’s local server

In LM Studio:

```text
1. Load your model
2. Go to the Server tab
3. Start the local server
```

The default endpoint is usually:

```text
http://localhost:1234/v1
```

Test it:

```powershell
Invoke-RestMethod -Uri "http://localhost:1234/v1/models" -Method Get
```

You should see your loaded model.

---

## 2. Install the OpenAI Python client

LM Studio’s server is OpenAI-compatible, so this works well:

```powershell
pip install openai
```

---

## 3. Create a local inspector script

Create this file:

```powershell
X:\source\facial-recognition\tools\lmstudio_inspector.py
```

Paste in:

```python
import json
import os
import sys
from pathlib import Path

from openai import OpenAI


# Scoped to your PoC.
ROOT = Path(os.environ.get("POC_ROOT", r"X:\source\facial-recognition")).resolve()

LMSTUDIO_BASE_URL = os.environ.get("LMSTUDIO_BASE_URL", "http://localhost:1234/v1")
LMSTUDIO_MODEL = os.environ.get("LMSTUDIO_MODEL", "local-model")
LMSTUDIO_API_KEY = os.environ.get("LMSTUDIO_API_KEY", "lm-studio")

MAX_CHARS = 20_000

ALLOWED_EXTENSIONS = {
    ".py",
    ".md",
    ".txt",
    ".yaml",
    ".yml",
    ".json",
    ".toml",
    ".cfg",
    ".ini",
    ".csv",
    ".html",
    ".css",
    ".js",
    ".ts",
}

EXCLUDED_DIRS = {
    ".git",
    ".venv",
    "venv",
    "__pycache__",
    "node_modules",
    "dist",
    "build",
    ".idea",
    ".vscode",
    "models",
    "weights",
    "checkpoints",
}

client = OpenAI(
    base_url=LMSTUDIO_BASE_URL,
    api_key=LMSTUDIO_API_KEY,
)


def truncate(text: str, limit: int = MAX_CHARS) -&gt; str:
    text = text.strip()
    if len(text) &lt;= limit:
        return text
    return text[:limit] + f"\n...[truncated {len(text) - limit} chars]..."


def is_allowed_file(path: Path) -&gt; bool:
    if not path.is_file():
        return False

    if path.suffix.lower() not in ALLOWED_EXTENSIONS:
        return False

    for part in path.parts:
        if part in EXCLUDED_DIRS:
            return False

    return True


def list_files() -&gt; str:
    files = []

    for path in sorted(ROOT.rglob("*")):
        if is_allowed_file(path):
            files.append(
                {
                    "path": path.relative_to(ROOT).as_posix(),
                    "size_bytes": path.stat().st_size,
                }
            )

    return truncate(json.dumps(files, indent=2))


def read_file(relative_path: str) -&gt; str:
    target = (ROOT / relative_path).resolve()

    if not target.is_relative_to(ROOT):
        raise PermissionError("Path is outside the allowed project root.")

    if not is_allowed_file(target):
        raise ValueError(f"File type or path is not allowed: {relative_path}")

    content = target.read_text(encoding="utf-8", errors="ignore")
    return truncate(content)


def search_code(query: str, max_results: int = 100) -&gt; str:
    q = query.lower()
    results = []

    for path in sorted(ROOT.rglob("*")):
        if len(results) &gt;= max_results:
            break

        if not is_allowed_file(path):
            continue

        try:
            content = path.read_text(encoding="utf-8", errors="ignore")
        except Exception:
            continue

        for line_number, line in enumerate(content.splitlines(), start=1):
            if q in line.lower():
                results.append(
                    {
                        "path": path.relative_to(ROOT).as_posix(),
                        "line": line_number,
                        "text": line.strip(),
                    }
                )

    return truncate(json.dumps(results, indent=2))


def execute_action(action: dict) -&gt; str:
    name = action.get("action")

    if name == "list":
        return list_files()

    if name == "read":
        return read_file(action.get("path", ""))

    if name == "search":
        return search_code(action.get("query", ""), int(action.get("max_results", 100)))

    raise ValueError(f"Unknown action: {name}")


SYSTEM_PROMPT = """
You are a local code reviewer running on the user's machine.
You can inspect a project through a local tool executor.

The project root is:
X:\\source\\facial-recognition

You must output exactly one JSON object per turn.

Allowed actions:

List files:
{"action": "list"}

Read a file:
{"action": "read", "path": "main.py"}

Search code:
{"action": "search", "query": "InferenceSession", "max_results": 50}

Give final answer:
{"action": "answer", "text": "Your final review here"}

Rules:
- Use relative paths only.
- Do not invent file contents.
- First list files.
- Read the important files.
- Then search for key terms such as:
  - onnxruntime
  - openvino
  - torch
  - cuda
  - dml
  - npu
  - gpu
  - mediaPipe
  - face
  - track
  - emotion
  - valence
  - arousal
  - confidence
  - model
  - infer
- Finally answer with a concrete review.
"""


def parse_json_object(text: str) -&gt; dict:
    start = text.find("{")
    end = text.rfind("}")

    if start == -1 or end == -1:
        raise ValueError("No JSON object found in model response.")

    return json.loads(text[start : end + 1])


def ask_model(messages: list) -&gt; str:
    response = client.chat.completions.create(
        model=LMSTUDIO_MODEL,
        messages=messages,
        temperature=0.2,
        max_tokens=2000,
    )

    return response.choices[0].message.content


def run_agent(initial_prompt: str, max_steps: int = 12) -&gt; str:
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": initial_prompt},
    ]

    for step in range(max_steps):
        model_output = ask_model(messages)
        messages.append({"role": "assistant", "content": model_output})

        try:
            action = parse_json_object(model_output)
        except Exception:
            # If the model stops producing JSON, treat its text as the final answer.
            return model_output.strip()

        if action.get("action") == "answer":
            return action.get("text", model_output.strip())

        try:
            tool_result = execute_action(action)
        except Exception as exc:
            tool_result = f"ERROR: {exc}"

        messages.append(
            {
                "role": "user",
                "content": (
                    "TOOL_RESULT:\n"
                    f"{tool_result}\n\n"
                    "Continue. Output exactly one JSON action object."
                ),
            }
        )

    return "Maximum steps reached without final answer."


if __name__ == "__main__":
    default_prompt = (
        "Inspect this facial expression / emotion recognition proof of concept. "
        "Compare it to best practices for real-time live face streams. "
        "Focus especially on: face tracking, temporal modeling, NPU/GPU usage, "
        "inference runtime, provider selection, latency, confidence handling, "
        "multiple faces, robustness, privacy, and code structure. "
        "Give concrete file-level recommendations."
    )

    prompt = sys.argv[1] if len(sys.argv) &gt; 1 else default_prompt
    print(run_agent(prompt))
```

---

# 4. Set the model name

Find your model ID from LM Studio or from:

```powershell
Invoke-RestMethod -Uri "http://localhost:1234/v1/models" -Method Get
```

Then set the environment variable:

```powershell
$env:LMSTUDIO_MODEL = "your-model-name-here"
```

If your LM Studio server requires an API key, set:

```powershell
$env:LMSTUDIO_API_KEY = "some-key"
```

If not, leave it as:

```powershell
$env:LMSTUDIO_API_KEY = "lm-studio"
```

---

# 5. Run the inspector

From PowerShell:

```powershell
cd X:\source\facial-recognition
python tools\lmstudio_inspector.py
```

Or with a custom prompt:

```powershell
python tools\lmstudio_inspector.py "Review the face tracking and NPU/GPU inference pipeline. Give concrete recommendations."
```

This script gives the local model three tools:

```text
list
read
search
```

and lets it produce a final answer.

---

# Why this is the right pattern

This gives you a safe architecture:

```text
LM Studio
  → local model inference

Python process
  → actual filesystem access
  → scoped to X:\source\facial-recognition
  → executes list/read/search
  → returns results to model
```

The model does not magically touch your disk. The Python process does, under your control.

That is much better than giving a model unrestricted shell access.

---

# If you want a more permanent solution

You have a few good options.

## Option A — Use an MCP filesystem server

If your LM Studio setup or client supports MCP, connect a filesystem MCP server scoped to:

```text
X:\source\facial-recognition
```

Then the model can call:

```text
list_directory
read_file
search
```

through MCP.

This is one of the cleaner long-term approaches.

---

## Option B — Use a local AI coding agent

Use an agent that can run tools and point it at LM Studio’s local server.

Examples:

```text
Cline
Roo Code
Continue
Aider
OpenHands
Open WebUI with tools
LangChain / LlamaIndex agent
```

The agent provides the tools:

```text
read_file
write_file
list_directory
run_command
search
```

LM Studio provides the model.

That combination is usually the best local setup.

---

## Option C — Use LM Studio’s own file/context features, if present

Some LM Studio versions or companion UIs may let you attach files or add project context. If your UI has:

```text
Attach file
Upload file
Add context
Project folder
MCP server
Plugin
```

then that may be the simplest path.

But the core local server alone does not necessarily give the model direct filesystem access.

---

# The key point

Even though LM Studio is local, the model still needs an execution layer.

```text
Local model ≠ local file access
```

You need:

```text
Local model
+ local tool executor
+ scoped filesystem permission
```

Once that bridge exists, the model can inspect your PoC properly.

For your case, the fastest practical path is:

```text
1. Start LM Studio server
2. Run the Python inspector script above
3. Let it list/read/search X:\source\facial-recognition
4. Let it produce the review
```

That will let the local model actually examine the code instead of just reasoning abstractly about it.

### User
This is a Enriched Context Generation scenario.

The following content was found in the files provided by the user.


** AGENTS.md full content **

# Facial-Recognition

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it.

Full background, history, measured tuning values and gotchas: **see `PROJECT_CONTEXT.md`.**
User-facing docs: `face-recognizer/README.md`.

## Layout

- `face-tracker/index.html` — original standalone MediaPipe tracker; double-click to open.
- `face-recognizer/` — the Python project. All programs share `face-recognizer/data/`:
  - `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`).
  - `labels.py` — labeling engine: per-face labels (`LabelDB`), background photo `Importer`,
    auto-naming, duplicate-photo detection, undo history.
  - `label_tool.py` + `label_ui.html` / `label.bat` — labeling web app on `http://127.0.0.1:8765`.
    Chris calls this "the recognizer".
  - `tracker.py` + `tracker_ui.html` / `recognize.bat` — live tracker on `http://127.0.0.1:8766`.
  - `localguard.py` — request checks for both local servers (Host, `X-Face-Tool: 1`, Origin).
  - `setup.bat` — installs Python 3.12 (winget) and `requirements.txt`.
  - `models/` — YuNet (detect), ArcFace R50 (recognize; also shipped split as `.part0`–`.part8`, rejoined
    by `_join_parts()`), SFace (fallback), eDifFIQA (quality), expression model, AdaFace (evaluated, unused),
    `npu/` (static ArcFace + VitisAI config/cache).
  - `tools/` — `eval_models.py`, `hw_bench.py`, `expr_check.py`, `get_adaface.py`, `import_bench.py`.

## Running

```
face-recognizer\setup.bat        # once
face-recognizer\recognize.bat    # live tracker  (port 8766)
face-recognizer\label.bat        # labeling tool (port 8765)
python face_id.py --help         # CLI pipeline (run from face-recognizer\)
```

Use the Python 3.12 the .bat files pick (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`). The
`python` on PATH (3.15) does not have OpenCV/onnxruntime installed.

## Conventions and constraints

- Keep `face-recognizer/README.md` in sync with any user-visible change.
- Thresholds are **measured values** — don't change them without re-measuring on `data/`:
  `face_id.py` (`REC_MODELS[...]["match"/"cluster"]`, `Q_ENROLL`, `Q_EXPR`, `MAX_TURN`),
  `labels.py` (`AUTO`, `MARGIN`, `TOPK`, `DUP_BITS`), and `tracker_ui.html` (strictness default 0.35,
  `X_SURE`, `X_MARGIN`, `X_MIN_VOTES`, vote windows).
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference).
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Expression is display-only and never stored; keep README wording honest ("looks", not "feels").
- InsightFace ArcFace weights are non-commercial; note this if the project is ever redistributed.


** end of AGENTS.md **



** CLAUDE.md full content **

# Facial-Recognition

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it.

Full background, history, measured tuning values and gotchas: **see `PROJECT_CONTEXT.md`.**
User-facing docs: `face-recognizer/README.md`.

## Layout

- `face-tracker/index.html` — original standalone MediaPipe tracker; double-click to open.
- `face-recognizer/` — the Python project. All programs share `face-recognizer/data/`:
  - `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`).
  - `labels.py` — labeling engine: per-face labels (`LabelDB`), background photo `Importer`,
    auto-naming, duplicate-photo detection, undo history.
  - `label_tool.py` + `label_ui.html` / `label.bat` — labeling web app on `http://127.0.0.1:8765`.
    Chris calls this "the recognizer".
  - `tracker.py` + `tracker_ui.html` / `recognize.bat` — live tracker on `http://127.0.0.1:8766`.
  - `localguard.py` — request checks for both local servers (Host, `X-Face-Tool: 1`, Origin).
  - `setup.bat` — installs Python 3.12 (winget) and `requirements.txt`.
  - `models/` — YuNet (detect), ArcFace R50 (recognize; also shipped split as `.part0`–`.part8`, rejoined
    by `_join_parts()`), SFace (fallback), eDifFIQA (quality), expression model, AdaFace (evaluated, unused),
    `npu/` (static ArcFace + VitisAI config/cache).
  - `tools/` — `eval_models.py`, `hw_bench.py`, `expr_check.py`, `get_adaface.py`, `import_bench.py`.

## Running

```
face-recognizer\setup.bat        # once
face-recognizer\recognize.bat    # live tracker  (port 8766)
face-recognizer\label.bat        # labeling tool (port 8765)
python face_id.py --help         # CLI pipeline (run from face-recognizer\)
```

Use the Python 3.12 the .bat files pick (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`). The
`python` on PATH (3.15) does not have OpenCV/onnxruntime installed.

## Conventions and constraints

- Keep `face-recognizer/README.md` in sync with any user-visible change.
- Thresholds are **measured values** — don't change them without re-measuring on `data/`:
  `face_id.py` (`REC_MODELS[...]["match"/"cluster"]`, `Q_ENROLL`, `Q_EXPR`, `MAX_TURN`),
  `labels.py` (`AUTO`, `MARGIN`, `TOPK`, `DUP_BITS`), and `tracker_ui.html` (strictness default 0.35,
  `X_SURE`, `X_MARGIN`, `X_MIN_VOTES`, vote windows).
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference).
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Expression is display-only and never stored; keep README wording honest ("looks", not "feels").
- InsightFace ArcFace weights are non-commercial; note this if the project is ever redistributed.


** end of CLAUDE.md **



** PROJECT_CONTEXT.md full content **

# Facial-Recognition — Project Context Export

Exported 2026-10-29 from the Claude Project **"Facial-Recognition"**
(*Detect known faces and provide for the management and labeling of new or unknown faces.*).
Revised the same day to match the code and data as they now stand.

This file collects everything established across the Project's chats so a coding assistant or a human
opening the repo in VS Code has the full background: what was built, why, where it lives, the measured
tuning values, the owner's stated preferences, and the environment gotchas that cost time before.

---

## 1. What this is

A fully local face-recognition system for a USB webcam, built for Chris:

1. **Face tracker** — the original standalone browser page (MediaPipe face detector).
2. **Face recognizer** — a Python pipeline that learned specific people from Chris's personal photo
   library and names them live on the webcam.
3. **Labeling tool** — a local browser app for adding photos, confirming suggested names, naming new
   people, ignoring false positives and fixing mistakes.
4. **Live tracker** — a local browser app that overlays each person's name, match confidence, and
   (optionally) a facial-expression reading directly on their face.

Nothing leaves the machine. No photos or face data are sent anywhere.

---

## 2. Where it lives (Windows)

```
X:\source\facial-recognition\
├── CLAUDE.md, PROJECT_CONTEXT.md
├── face-tracker\                 original standalone HTML tracker (MediaPipe), double-click index.html
│   └── index.html
└── face-recognizer\              the Python project (everything below shares one data\ folder)
    ├── face_id.py                core library + CLI: scan / cluster / name / people / recognize / reembed / migrate
    ├── labels.py                 labeling engine: LabelDB (per-face labels, undo), Importer (background scan)
    ├── label_tool.py             labeling web app  → http://127.0.0.1:8765   (launch: label.bat)
    ├── label_ui.html             labeling page (served by label_tool.py)
    ├── tracker.py                live tracker server → http://127.0.0.1:8766 (launch: recognize.bat)
    ├── tracker_ui.html           live tracker page (served by tracker.py)
    ├── localguard.py             refuse_reason(): request checks shared by both servers
    ├── setup.bat                 installs Python 3.12 via winget, then pip installs requirements.txt
    ├── recognize.bat             starts tracker.py (checks cv2/numpy are installed first)
    ├── label.bat                 starts label_tool.py
    ├── requirements.txt          opencv-python, numpy, scikit-learn, pillow, pillow-heif, onnxruntime-directml
    ├── README.md                 user-facing docs; kept in sync with code changes
    ├── tools\
    │   ├── eval_models.py        ArcFace vs AdaFace vs both, on clean/blurred/tiny/motion faces → data\model_eval.json
    │   ├── hw_bench.py           CPU vs GPU vs NPU timing; compiles the NPU model → data\hw_bench.json
    │   ├── expr_check.py         sanity check for the expression model
    │   ├── get_adaface.py        downloads the official AdaFace IR-101 weights and converts them to ONNX
    │   └── import_bench.py       photo-import timing
    ├── models\
    │   ├── face_detection_yunet_2023mar.onnx           YuNet detector (OpenCV Zoo)
    │   ├── arcface_w600k_r50.onnx (+ .part0–.part8)    ArcFace recognizer (174 MB), split copy rejoined by _join_parts()
    │   ├── face_recognition_sface_2021dec.onnx (+ .part0/.part1)   SFace, fallback recognizer (38.7 MB)
    │   ├── ediffiqa_tiny_jun2024.onnx                  eDifFIQA face quality (optional)
    │   ├── facial_expression_recognition_mobilefacenet_2022july.onnx   7-class expression model (optional)
    │   ├── adaface_ir101_webface12m.onnx, adaface_src\  AdaFace, evaluated and not used
    │   └── npu\                                        static batch-1 ArcFace, vitisai_config.json, compile cache
    └── data\                     produced by the tools; shared by all programs
        ├── faces.npz             every face found: permanent ID (fid), photo, box, face print, quality, thumbnail
        ├── photos.json           every photo read, keyed by content hash: path, size, mtime, picture fingerprint,
        │                         face count, and which photo a skipped copy duplicates
        ├── labels.json           label of each face, keyed by face ID (see §3)
        ├── history.jsonl         every labeling change with its time (powers Undo)
        ├── backups\              labels-&lt;time&gt;.json from each labeling-tool start (last 20);
        │                         legacy-&lt;time&gt;\ = the old groups.json/names.json/scanned.json
        ├── people.npz            face prints of named people — what the tracker and `recognize` use
        ├── settings.json         photo folders that have been added
        ├── sheets\               contact sheets written by `cluster` (group_###.jpg, overview_*.jpg)
        └── model_eval.json, hw_bench.json   results from the tools above
```

`face-recognizer\data_backup_20261029\` is a full copy of `data\` from before the per-face migration
(old format, 3.8 MB faces.npz).

Earlier in the project the two halves lived at `X:\source\face-tracker` and `X:\source\face-recognizer`;
they were later consolidated under `X:\source\facial-recognition\` as the two subfolders above.

**Vocabulary Chris uses** (recorded so the right thing gets launched):
- "the recognizer" = the **labeling tool** (`label.bat`, port 8765), *not* `recognize.bat`.
- "the tracker" = the live tracker (`recognize.bat` → `tracker.py`, port 8766).

Hardware: a simple USB webcam; AMD Ryzen AI Max+ 395 PC (16-core CPU, Radeon 8060S GPU, XDNA 2 NPU).
Photo source for training: `E:\Users\Chris\iCloud\Photos`.

---

## 3. Data model and pipeline

### Per-face labels (labels.py)

Every face has a **permanent ID**: its photo's content hash + where the face is in the photo
(`face_key()`, e.g. `6edd140b4d6dc4a9-94-848`). Labels are stored on the face, so regrouping, rescanning,
or moving/renaming photos never loses them:

```
{"&lt;face id&gt;": {"n": "Chris", "by": "me"}, ...}
  n    the person's name, or "#ignore" (stranger, artwork, not a face: never recognized)
  by   "me" (named by Chris) or "auto" (named automatically because the model was very sure)
  not  names this face is NOT, so they are never suggested again
  ok   a "might be mislabeled" warning was checked and the name kept
  s    the model's score when the face was named automatically
```

Groups are no longer stored. For unnamed faces the engine works out on the fly:
- **suggested** — the model thinks it knows who it is (score ≥ `MATCH`): Chris confirms or rejects.
- **unknown** — nobody known is close: shown in groups of look-alikes to name or ignore. Groups that come
  within `HINT` (`MATCH − 0.12`) of a known person get a "looks like …" hint.

Every change is logged to `history.jsonl` and can be undone. `people.npz` is rebuilt from the labels.

### Labeling tool (label_tool.py + label_ui.html)

- **Add photos** runs the `Importer` in the background on the GPU, subfolders included. It skips photos
  already read (by content hash, so renamed/moved photos are recognized) and copies of photos (re-saved or
  resized; picture fingerprints within `DUP_BITS` = 12 bits **and** matching faces). New faces that clearly
  match a named person are **named automatically**.
- Views: **Suggestions** (per person, least certain first), **New faces** (look-alike groups),
  **Named automatically** (review, least certain first), **Checks** (named faces that look more like someone
  else, people with fewer than `FEW` = 5 clear faces, skipped duplicates), **People** (all faces of a person,
  least typical first; **Rename or merge**).
- Double-click / Space shows the whole photo with everyone boxed and named. Keys: A accept, N name,
  R not this person, X ignore, Z undo, Esc clear selection, Shift-click range.
- Faces too blurry to learn from are hidden unless **Show blurry faces** is on.
- API: GET `/api/summary`, `/api/fids`, `/api/suggested`, `/api/unknown`, `/api/auto`, `/api/person`,
  `/api/checks`, `/api/import`, `/api/photoinfo/…`; POST `/api/act`, `/api/rename`, `/api/undo`,
  `/api/import`, `/api/import/stop`, `/api/autoname`.

### CLI (face_id.py)

```
scan       find every face in a photo folder (only new photos are read)  — same as Add photos
cluster    group unnamed look-alike faces, write contact sheets to data\sheets (--min 4)
name       name one or more cluster groups, or --ignore them
recognize  name people live on a webcam, or in photos / videos on disk
reembed    recompute face prints from the stored thumbnails after switching model (no photo rescan)
migrate    upgrade an old groups/names database to permanent face IDs + per-face labels
people     list who it knows
```

```
python face_id.py scan "E:\Users\Chris\iCloud\Photos"
python face_id.py cluster
python face_id.py name 4 9 15 Chris          # name groups 4, 9 and 15
python face_id.py name 7 --ignore            # not someone to recognize
python face_id.py people
python face_id.py recognize --camera 0
python face_id.py recognize "D:\some\photos" --out tagged
python face_id.py reembed
python face_id.py migrate "&lt;photo folder&gt;"   # also runs automatically the first time Face Labels starts
```

`cluster` group numbers are only valid until the next `cluster`; since labels are stored per face, naming a
group labels its faces permanently. The labeling tool is the preferred way to name faces.

Image handling: `load_image()` applies EXIF rotation, decodes HEIC/HEIF (pillow-heif), shrinks to
`MAX_SIDE`. Supported: jpg/jpeg/png/webp/bmp/heic/heif/gif/tif/tiff; videos mov/mp4/m4v/avi/mkv.

---

## 4. Models and measured tuning values

Detection: **OpenCV YuNet** (`cv2.FaceDetectorYN`, CPU). Quality: **eDifFIQA** tiny.
Recognition: **ArcFace** (ResNet-50, WebFace600K, from InsightFace buffalo_l) when installed, else **SFace**.
`FACE_MODEL=sface_2021dec` forces the old model. InsightFace pretrained models are non-commercial;
commercial use needs a license. Faces are aligned to the 5-point 112×112 template (`align()`).

```python
MAX_SIDE  = 1280   # photos shrunk to this before detection
DET_SCORE = 0.85   # detector confidence needed to keep a face
MIN_FACE  = 40     # px; smaller faces too blurry to recognize
Q_ENROLL  = 0.30   # faces below this eDifFIQA quality are not used to learn a person
Q_EXPR    = 0.25   # faces below this quality aren't scored for expression
MAX_TURN  = 0.35   # nose offset / eye distance (~35°); more turned faces aren't scored for expression

REC_MODELS = {
    "arcface_w600k_r50": dict(file="arcface_w600k_r50.onnx",              dim=512, match=0.35, cluster=0.70),
    "sface_2021dec":     dict(file="face_recognition_sface_2021dec.onnx", dim=128, match=0.45, cluster=0.58),
}
# match   = cosine similarity needed to call a face a known person
# cluster = cosine distance for grouping faces as one person

# labels.py
AUTO   = 0.45   # auto-name a new face at or above this score ...
MARGIN = 0.10   # ... if the next most likely person scores at least this much lower
TOPK   = 5      # a person's score = mean similarity of their 5 closest face prints
```

Measured on Chris's labeled faces:
- **ArcFace @ 0.35**: 100% of labeled faces correct, 0% of strangers named; on simulated blurry webcam
  frames 76% correct vs 44% for SFace. Clustering at 0.70 put Chris in 2 groups instead of 4.
- **SFace @ 0.45**: 99.3% correct, 0% strangers named (0.40 named 4.4% of strangers).
- **Auto-naming (AUTO 0.45 + MARGIN 0.10)**: named 89% of labeled faces with no mistakes; no stranger
  scored above 0.35.
- **AdaFace IR-101** (WebFace12M, `tools/get_adaface.py`): 8.5 ms/face on GPU, but 95.5% vs ArcFace 99.4%
  on clean labeled faces, about the same on blurry; ArcFace+AdaFace combined 97.7%. Kept ArcFace
  (`tools/eval_models.py`, results in `data/model_eval.json`).
- Matching rule: a face is labeled with the known person whose 5 closest face prints are most similar,
  if similarity ≥ the threshold; otherwise "Unknown".

### Compute (FACE_DEVICE = auto | gpu | npu | cpu)

| Processor | ArcFace, one face | Notes |
|---|---|---|
| GPU (Radeon 8060S, onnxruntime DirectML) | 2.6–3.6 ms | `auto` default; identical results to CPU |
| CPU (OpenCV DNN) | ~12 ms | fallback |
| NPU (Ryzen AI 1.8 VitisAI, BF16) | ~25 ms | 99.97% identical; first start compiles ~6 min (342 s) |

The recognizer, quality and expression models use ONNX Runtime when available; YuNet stays on the CPU.
The NPU needs the Ryzen AI env's Python (`X:\ProgramData\miniconda3\envs\ryzen-ai-1.8.0\python.exe`) and
the static model in `models\npu\`; `tools/hw_bench.py` compiles it and re-measures all three
(`data/hw_bench.json`). `tracker.py` prints `Recognition model: &lt;name&gt; on &lt;device&gt;` at startup.

---

## 5. Live tracker (tracker.py + tracker_ui.html)

- Browser page served at `http://127.0.0.1:8766`; the page grabs webcam frames in the browser and POSTs
  JPEGs to `/api/recognize` (GET `/api/people` lists names). The server detects at 640 px in one pass and
  returns box, keypoints, name, similarity, quality and expression for each face.
- `Engine` reloads `people.npz` whenever its mtime changes, so names saved in the labeling tool appear
  automatically. Each request takes a snapshot of the identifier and passes its own threshold, so two tabs
  with different strictness don't interfere; detection runs under a lock (OpenCV isn't thread-safe).
- Each face gets a box; **name + match %** are drawn on the face near the **top of the head** (Chris's
  explicit preference — not above the bounding box). Unnamed people show as **Unknown**.
- **Match strictness** slider 0.20–0.60, default **0.35** (the measured ArcFace setting; the earlier 0.45
  was the SFace-era value). Per-track name voting over the last **7** frames, each vote weighted by face
  quality (similarity breaks ties), so sharp frames outvote blurry ones.
- **Display**: mirror view, keypoints, motion trail, show or blur unknown faces, smoothing.
- **Camera**: pick webcam by name; auto-starts the last-used camera on page load (localStorage) and
  stops auto-starting if the user explicitly clicks **Stop camera**. **Open video** tracks a file.
- **Known people** card is a collapsible `&lt;details&gt;`, collapsed by default, remembers open/closed state,
  shows a count like "2 of 26" (on-camera of known). Live readout still lists detected names.
- Hardening (`localguard.refuse_reason()`, used by both servers → 403 otherwise): Host must be
  `127.0.0.1:&lt;port&gt;` or `localhost:&lt;port&gt;` (defeats DNS rebinding); POSTs must carry `X-Face-Tool: 1` and,
  if an Origin is sent, come from the tool's own page. `BrokenPipeError`/`ConnectionResetError` on write are
  swallowed (page closed/reloaded mid-request); a Windows port fix sits at the top of `main()`.

---

## 6. Expression (added 2026-10-29)

Asked "how hard would it be to detect emotion?" — answer: easy to get *something*, hard to get it
*good*. Implemented as expression-on-the-face (not felt emotion); README wording is deliberately honest
("this face currently looks …"). 7 classes: angry, disgust, fearful, happy, neutral, sad, surprised
(OpenCV Zoo MobileFaceNet, Apache 2.0, 88% on RAF-DB).

Implementation (`face_id.py` → `Faces.expression()`, `Faces._optional()`, `head_turn()`;
`tracker.py` → gating in `Engine.recognize()`; `tracker_ui.html` → section `// ---- expression`):

- **Only readable faces are scored**: skip frames with quality &lt; `Q_EXPR` (0.25) or head turned more
  than `MAX_TURN` (~35°). Blur/profile makes the model *wrong* (usually "sad"/"neutral"), not unsure.
- **Score twice** (as-is + mirrored), average. Mirrored views agree 93% of the time on stored faces;
  both runs ≈ 0.7 ms per face on GPU.
- **Frames vote** (tracker_ui.html): last 10 scored frames (~1 s), weighted by quality; report a label
  only if there are at least 3 votes (`X_MIN_VOTES`), it has ≥ 45% of the vote (`X_SURE`) and leads the
  runner-up by 12 points (`X_MARGIN`); otherwise shown dimmed with "?". After 10 unscored frames in a row
  the reading is dropped rather than left stale. 95% of stored faces give a clear winner.
- **Calibrate to each person** (on by default): slow running average of each named person's readings,
  kept in the browser (localStorage key `ft-usual`); trusted after ~100 frames. Readings that match the
  usual look are tagged **usual** (dim); departures show bright. **Forget learned usual looks** clears it.
  Functions: `learnUsual`, `readExpression`.
- Reliable labels: happy, surprised, neutral. Fearful/disgust are rare in training data and most
  confused; angry/sad are often just a resting face. Photo-library distribution: ~40–45% neutral,
  ~40–45% happy, rest thin.
- Expression is never stored; display only. The model is optional (the tracker says so at startup if
  missing). UI toggles: **Show expression**, **Calibrate to each person**.
- `python tools\expr_check.py` scores a sample of stored faces (ms/face, label spread, mirror agreement)
  and runs one tracker request on a stored photo — confirms the model is installed and working.
- Status at end of session: Chris confirmed "It is working now!" and the collapsible Known-people card
  "worked!!".

---

## 7. Training data and history

Current data (2026-10-29):
- **5,702 photos** read from `E:\Users\Chris\iCloud\Photos`, of which 112 were skipped as copies.
- **1,551 faces** in `faces.npz`; **786 labeled**: 456 by Chris, 328 named automatically, 231 of them
  `#ignore`.
- **26 named people**; `people.npz` holds 544 face prints.

History:
- The first scan read **1,936 photos** → **638 faces** after fixing HEIC decoding and adding
  close-up/selfie handling. That listing truncated at 2,000 entries, so ~60 photos were missed at first.
- Faces were clustered; Chris named **14 people** in the first pass, and more later in the labeling tool.
- Groups that were artwork, video-game characters, tattoos, sculptures, or ambiguous images were ignored
  at Chris's request.
- The database was then migrated from group-based labels (`groups.json`/`names.json`/`scanned.json`) to
  permanent face IDs with per-face labels; the old files are in `data\backups\legacy-20261029-080425\`
  and `data_backup_20261029\`.

Timeline of the Project's chats:

| Date (2026) | Chat | What happened |
|---|---|---|
| ~Oct 26 | *Simple face tracking app* | HTML MediaPipe tracker → camera selector → CDN fix → Python recognizer (YuNet+SFace) trained on iCloud photos → clustering/naming → separate labeling tool → separate live tracker with on-face overlay → setup/launch .bat files → ArcFace + eDifFIQA + DirectML → auto-start camera → tracker hardening. |
| Oct 29 08:04 | — (not in the export; dated by `data\backups\`) | Migration to per-face labels. By the end of the day the labeling tool also had import, suggestions, auto-naming, duplicate detection, checks and undo, and NPU/AdaFace had been evaluated; when each was added isn't recorded. |
| Oct 29 13:37 | *Labeling tool launch* | Launched tracker and labeling tool on the PC; clarified "the recognizer" = labeling tool. |
| Oct 29 14:06 | *Emotion detection from face tracking* | Expression feature (gating, mirrored scoring, voting, per-person calibration), README update, collapsible Known-people card, opened repo in VS Code. |
| Oct 29 16:37 | *Export command* | `/export` attempt; led to this document. |

---

## 8. Chris's stated preferences and decisions (keep honoring these)

- The labeling/exclusion tool must be a **separate program** from the tracker.
- Name overlay goes **on the face / top of head**, not above the bounding box ("this is MUCH better").
- Ignore faces that are tattoos, art, sculptures, game characters.
- When something needs running on the PC, **run it** rather than giving manual instructions.
- Keep the README truthful about what expression detection can and can't do.
- Known-people list collapsed by default; live readout keeps names visible.
- Thresholds (match strictness default 0.35, `AUTO`/`MARGIN`, expression gates and votes) are measured
  values — don't change them without re-measuring.

---

## 9. Environment gotchas (learned the hard way)

Python on this machine:
- The `python` on PATH is 3.15 (`C:\Python315`) and has **no** OpenCV/onnxruntime. The project's libraries
  are in Python 3.12 (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`), which the .bat files prefer
  because other apps add newer Pythons that `py -3` would pick. Use that interpreter for CLI/tools.

Webcam / OpenCV on this Windows machine:
- `cv2.CAP_DSHOW` alone didn't reliably open the USB webcam; iterate `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`.
- Retry `cap.read()` up to ~10 times with short sleeps; some webcams return empty frames while starting.
- Camera index 1 = the Pixel phone's camera via Phone Link, not the USB webcam. Pick cameras by name in the browser UI.

Claude ↔ PC tooling (relevant if continuing with Claude's device tools):
- `device_commit_files` fails above ~30 MB → large models are shipped as 20 MB `.partN` files
  (ArcFace `.part0`–`.part8`, SFace `.part0`/`.part1`) and rejoined by `_join_parts()` in `face_id.py` on
  first run.
- `device_list_dir` truncates at 2,000 entries.
- Claude's browser tools can't open `file://` URLs — hence both apps are tiny local HTTP servers.
- Staging files in batches of ~50 lands under `/mnt/user-data/uploads/&lt;lastfolder&gt;/` with ~1 s lag per 50 MB.
- Opening a `cv2.imshow` window via computer-use needed full-tier access; the browser-based UI avoids this.

Code-editing convention used throughout: edits were applied with small Python scripts using a
`rep(old, new)` helper that asserts the target string occurs exactly once — safe for surgical changes.

---

## 10. Open items / ideas not yet done

None were explicitly committed to. Ideas floated but not built: MediaPipe Face Landmarker blendshapes as
a richer basis for expression than a 7-class classifier. If continuing, re-measure thresholds after any
model change (`python face_id.py reembed`, then `tools\eval_models.py` and `people`).


** end of PROJECT_CONTEXT.md **

Based on the content above, please provide a response to the user query.

User query: You can start with these .md files that give a very high level description of the solution:


### Assistant
User says "This is a Enriched Context Generation scenario. The following content was found in the files provided by the user. ... Based on content above, please provide a response to the user query. User query: You can start with these .md files that give a very high level description of the solution:"

We need respond based on provided MD files. Need likely acknowledge and provide initial high-level assessment/recommendations based on AGENTS.md, CLAUDE.md, PROJECT_CONTEXT.md. The user earlier wanted code review and NPU/GPU. Now they provided high-level docs. We should compare with previous best practices and provide specific recommendations based on docs. Need not inspect actual code yet, but can infer. Need maybe say "Thanks, these .md files give excellent context. Based on them, here's my initial assessment and what I want to inspect next." Need include recommendations. Need align with project: face recognition plus optional expression. The user's original query about identifying emotions in real-time streams. Their PoC has expression 7-class MobileFaceNet, quality gating, mirrored scoring, voting, calibration, NPU/GPU. Need compare to best practices: good: local privacy, localguard, measured thresholds, per-face labels, quality gating, voting, per-person calibration, honest wording. Areas to improve: expression is display-only 7-class discrete; best practice dimensional valence/arousal + AUs/landmarks temporal, multi-modal? For their use maybe expression optional. Need recommend inspect key files: face_id.py expression functions, tracker.py Engine.recognize, tracker_ui.html expression section, tools/expr_check.py, requirements, hw_bench, models. Need specific recommendations:

- Expression model: 7-class MobileFaceNet is reasonable baseline but limited; best approaches earlier suggested hybrid landmarks/AUs + temporal. Their implementation already does voting and calibration, which is good. But missing 3D/AU/landmark features; 7-class may confuse angry/sad/fearful/disgust. For real-time, maintain discrete but add confidence and abstention (they have X_MIN_VOTES etc). Good.
- Temporal: browser-side last 10 frames ~1s weighted by quality; good. Could add server-side per-track temporal state instead of browser only? Currently browser does voting; server returns per-frame expression. If multiple tabs or strictness, maybe okay. But for robust, maintain per-track state server-side? They have per-track name voting in browser. Could centralize in tracker server for consistency. But owner preference maybe browser.
- NPU/GPU: They have GPU DirectML 2.6-3.6ms, NPU 25ms, CPU 12ms. For expression, model MobileFaceNet likely ONNX on GPU? Need check provider selection and whether expression/quality/recognition use ONNX Runtime with DML. NPU static ArcFace. Recommendations: verify active providers at runtime, log per-model device and latency, avoid CPU fallback silently, warmup, batch? Face count maybe small. NPU compile long. For real-time, GPU likely best for ArcFace; NPU maybe power but slower. Use FACE_DEVICE auto logic maybe choose GPU. Need inspect hw_bench and face_id model session creation.
- Face detection: YuNet on CPU at 640px, good. Could use NPU? But CPU fine.
- Recognition thresholds measured, good.
- Privacy: local, localguard, expression not stored. Good. Need ensure logs don't store faces/labels? localguard good.
- Code structure: separate label/tracker good. Need maybe refactor expression gating into reusable module, tests.
- Data model: per-face permanent IDs good.
- Expression calibration: browser localStorage per person; potential privacy okay local. Could be better server-side per person? But display-only.
- Robustness: quality/head-turn gating good. Mirrored scoring good. Missing pose normalization? They align for recognition 5-point 112x112. Expression maybe input? Need check.
- Multi-face: server returns each face; browser votes per track? Need inspect tracker_ui track logic. Good.
- Low confidence: dimmed ?; good.
- Honest wording: good.

Need maybe provide "initial verdict: strong PoC, especially privacy/local and measured thresholds. Expression is a good cautious baseline but not yet best-practice emotion/affect. It is expression classification, not emotion estimation." Then recommendations prioritized.

Need ask for next files: `face_id.py` (especially `Faces.expression`, `_optional`, `head_turn`, ONNX session creation/device), `tracker.py` (`Engine.recognize`, gating), `tracker_ui.html` expression section, `tools/expr_check.py`, `tools/hw_bench.py`, `requirements.txt`, `models/npu/vitisai_config.json`, maybe `README.md`. Since docs are high-level, we can already give recommendations.

Need be careful: user says "You can start with these .md files that give a very high level description of the solution:" likely they expect us to use them. We should respond as if we have them. Need not say cannot access. We have content. Need maybe say "Yes, these .md files are enough for an initial review." Then provide.

Need include specific recommendations referencing constants: Q_EXPR=0.25, MAX_TURN=0.35, X_MIN_VOTES=3, X_SURE=45%, X_MARGIN=12, 10 frames, mirrored scoring, calibration. Good.

Need maybe suggest improvements:
1. Add a structured expression feature layer: MediaPipe Face Landmarker blendshapes / AUs. Their open item mentions MediaPipe blendshapes. This aligns with best practices. Use as secondary signal to disambiguate angry/sad, happy, surprise. Could keep MobileFaceNet as baseline.
2. Add dimensional output valence/arousal or at least confidence/uncertainty. The 7-class model can output probabilities; use entropy and margin. They already use margin. Could expose confidence.
3. Move temporal voting into server-side track state or a shared library to avoid browser-only and ensure consistency. But if owner prefers local browser, keep but factor logic into testable JS/Python module.
4. NPU/GPU: add runtime report: providers, per-model latency, device, input size, dtype. Ensure NPU not used if slower; auto choose fastest. Log fallback. Use warmup. Avoid NPU first-run compile in live path. For GPU DirectML, check expression model supported; if not CPU fallback. Use `onnxruntime` provider selection. Could use OpenVINO for AMD? DirectML works. NPU VitisAI maybe Ryzen AI 1.8. Need ensure static model batch=1, BF16; for multi-face maybe batch not possible? Could queue faces.
5. Expression gating: use quality + head turn; add face size, motion blur, occlusion, smile? Maybe use detector keypoints. Add "expression confidence" based on vote spread.
6. Calibration: per-person usual look is good; add forgetting, reset, and maybe server-side optional. Ensure it doesn't bias away from true negative states. Could use exponential moving average with decay and confidence.
7. Testing: add unit/integration tests for expression gating, voting, calibration; synthetic frames; regression suite with stored faces. `expr_check.py` good; extend to report per-class confusion, mirror agreement, quality stratified, head-turn stratified.
8. Documentation: README honest; add limitations and threshold provenance.
9. Security: localguard good; ensure no CORS, no remote, bind 127.0.0.1, check Host/Origin. Good.
10. Code: avoid global mutable state? Engine reloads people.npz mtime; detection lock; good. Could add async queue? But local small.

Need maybe compare to earlier "best approaches" table:
- Their face detection/tracking: browser MediaPipe? Actually tracker page grabs webcam frames and POSTs JPEGs; server detects YuNet. Face tracking? Browser per-track voting; maybe MediaPipe? Original face-tracker standalone. For live, server detection every frame? It POSTs each frame. Good.
- Feature extraction: recognition aligned; expression 7-class. No AUs. Recommendation.
- Temporal: browser voting. Good.
- Output: discrete labels, confidence-ish votes. Could add valence/arousal.
- NPU/GPU: measured, good.
- Privacy: excellent.

Need maybe mention "this is face recognition system with optional expression, not general real-time emotion identification; the best-practice emotion stack is broader, but your PoC is appropriately scoped and cautious." Good.

Need final with headings:
- "Yes — these .md files are a very good starting point"
- "Initial assessment"
- "What is already strong"
- "Where I would push it toward best practice"
- "NPU/GPU specific recommendations"
- "Expression-specific recommendations"
- "Files I want next"
- "Suggested review order"

Need maybe include a concise recommended architecture diff:
Current:
```
webcam frame -&gt; YuNet detect -&gt; ArcFace/SFace recognition -&gt; MobileFaceNet 7-class expression -&gt; browser votes/calibrate
```
Best-practice extension:
```
webcam frame -&gt; track -&gt; quality/pose gate -&gt; landmarks/blendshapes/AUs + expression CNN -&gt; temporal state -&gt; confidence/abstention -&gt; optional valence/arousal -&gt; UI
```

Need mention "don't change thresholds without remeasuring" and respect. Recommendations not to change thresholds arbitrarily.

Need maybe propose adding a "review gate" before changing expression model: run `expr_check.py`, `hw_bench.py`, `eval_models.py` after changes. Good.

Need maybe include specific code inspection questions:
- In `face_id.py`: how ONNX sessions are created? Are providers set? Does expression model run on GPU/NPU/CPU? Is `head_turn()` computed from 5-point? Is mirrored average implemented? Is `Q_EXPR` gating before/after inference? Is model optional? Is input normalized? Batch?
- In `tracker.py`: where gating occurs, how per-face results returned, thread safety, reload behavior.
- In `tracker_ui.html`: track association, vote windows, localStorage calibration, drawing, stale drop.
- In `tools/hw_bench.py`: how NPU compiled, how auto chooses device, whether results include provider and latency.

Need maybe ask to paste those files. But user said start with .md. We can say enough for high-level, but for specific recommendations need code. Yet we can already provide high-level recommendations.

Need maybe include "No secrets" okay.

Let's craft final. Ensure not claim we read actual code beyond docs. Say "based on the .md files". Good.

Need maybe mention "AGENTS.md and CLAUDE.md are duplicate; maybe keep one canonical to avoid drift." Good recommendation. Since AGENTS.md and CLAUDE.md identical. Could say consolidate or generate one from other. PROJECT_CONTEXT is rich. Good.

Need maybe mention "README sync" good.

Need maybe mention "duplicate docs can drift; make CLAUDE.md include AGENTS or symlink? On Windows maybe use one file." Good.

Need final oververbosity 9. Let's write.

Yes — those `.md` files are a very good starting point. They give enough context to make a meaningful initial assessment before opening the Python/HTML code.

Based only on `AGENTS.md`, `CLAUDE.md`, and `PROJECT_CONTEXT.md`, your PoC is already stronger than a typical “emotion detection” demo because it is scoped honestly, local-first, measured, and cautious about expression interpretation. It is not trying to claim “the person feels X”; it is trying to say “this face currently looks X,” with quality gating, voting, calibration, and optional display. That is the right posture.

Below is my high-level review and the specific recommendations I would make before we go into the code.

---

# 1. Overall assessment

Your system is effectively:

```text
local webcam
  → server-side face detection
  → face recognition / naming
  → optional expression classification
  → browser-side voting, smoothing, and per-person calibration
  → on-face overlay
```

That maps well to best practice for a local live-face tool, especially because you have:

- local-only processing
- DNS-rebinding / origin hardening
- measured thresholds
- separate labeling tool and live tracker
- per-face permanent IDs
- quality gating
- head-turn gating
- mirrored expression scoring
- temporal voting
- per-person “usual look” calibration
- honest README wording
- NPU/GPU/CPU benchmarking

For a PoC, this is a solid architecture.

The main area where it is still closer to “facial expression classification” than “robust real-time affect estimation” is the expression model itself. A 7-class MobileFaceNet classifier is a reasonable baseline, but the strongest real-time face-expression systems usually combine:

```text
expression CNN
+ facial geometry / landmarks / AUs
+ temporal state
+ confidence / abstention
+ optional valence/arousal dimensions
```

Your PoC already has temporal state and abstention-like behavior. The missing piece is richer expression representation, especially facial geometry / Action Units / blendshapes.

---

# 2. What is already strong

## A. Honest framing of expression

This is one of the best parts of the project.

From the context:

&gt; Expression is display-only and never stored; keep README wording honest (“looks”, not “feels”).

That is exactly the right boundary.

The system should not claim:

```text
Chris is angry.
Chris is sad.
Chris is lying.
Chris is depressed.
```

It should claim:

```text
This face currently looks possibly frustrated.
Confidence is moderate.
Expression is optional and display-only.
```

That is much safer and more defensible.

---

## B. Quality and head-turn gating

You have:

```text
Q_EXPR  = 0.25
MAX_TURN = 0.35
```

and you skip expression scoring when the face is too blurry or too turned.

That is important because expression models often do not fail gracefully. A blurry or profile face may be classified confidently as “sad” or “neutral” even when the model has no meaningful evidence.

Your approach is better than blindly running the classifier on every frame.

---

## C. Mirrored scoring

You score the face twice:

```text
normal + mirrored
```

and average.

That is a good practical robustness trick, especially for a webcam view where mirroring can affect appearance.

The fact that you measured mirror agreement is also good.

---

## D. Temporal voting

You use:

```text
last 10 scored frames
weighted by quality
minimum votes
minimum certainty
margin over runner-up
stale-drop after unscored frames
```

That is much better than single-frame classification.

This is one of the key best-practice elements for real-time expression inference.

---

## E. Per-person calibration

You maintain a slow running average of each person’s usual expression reading, then tag readings that match the usual look as “usual” and show departures more brightly.

That is a very useful idea for a personal local system.

It helps with the fact that people have different resting faces. For example:

- one person may naturally have a more furrowed brow,
- another may have a more relaxed mouth,
- another may have a resting expression that a generic model reads as mildly sad or angry.

This is one of the strongest differentiators in your PoC.

---

## F. Local privacy model

The project is local-only:

```text
no data leaves the machine
local HTTP server on 127.0.0.1
localguard checks Host / Origin / X-Face-Tool
expression is not stored
labels are local
models are local
```

That is excellent for a webcam face tool.

The hardening in `localguard.py` is also better than many local web apps.

---

## G. Measured thresholds

You have measured values such as:

```text
ArcFace match = 0.35
SFace match = 0.45
AUTO = 0.45
MARGIN = 0.10
X_MIN_VOTES = 3
X_SURE = 45%
X_MARGIN = 12
```

And you explicitly say not to change them without re-measuring.

That is a very good engineering discipline.

A lot of face/emotion PoCs fail because thresholds are guessed. Yours are documented as measured.

---

# 3. Main gap: expression model is still relatively shallow

Your current expression approach is:

```text
MobileFaceNet 7-class classifier
  → quality gate
  → head-turn gate
  → mirrored scoring
  → browser voting
  → per-person calibration
```

That is a good baseline.

But for stronger real-time expression identification, I would move toward a hybrid representation:

```text
MobileFaceNet / expression CNN
+ face landmarks
+ blendshapes
+ Action Units
+ head pose
+ eye/mouth dynamics
+ temporal state
```

Your open item already mentions this:

&gt; MediaPipe Face Landmarker blendshapes as a richer basis for expression than a 7-class classifier.

That is the right next direction.

---

# 4. Specific recommendations

## Recommendation 1: Add a lightweight facial-geometry expression channel

Keep the 7-class MobileFaceNet model as a baseline, but add a second signal from face geometry.

Useful features:

```text
brow height
brow tilt
inner brow raise
outer brow raise
eye openness
eye tension
lid raise
cheek raise
mouth corner pull
mouth corner depress
mouth openness
jaw drop
lip tightness
head pitch
head yaw
head roll
blink rate
gaze direction if available
```

These can be extracted from:

- MediaPipe Face Landmarker
- OpenCV facial landmarks
- a 3DMM/FLAME model
- existing YuNet keypoints, if they are rich enough

You do not necessarily need a huge new model. A small classifier or rule layer over temporal landmark deltas could improve disambiguation.

For example:

| Signal | May indicate |
|---|---|
| strong brow lower + lip tightness | frustration / anger / concentration |
| cheek raise + mouth corner pull | happy / amused |
| lid raise + brow raise + mouth open | surprised |
| mouth corner depress | sad / disappointed |
| brow raise only | confused / inquiring / startled |
| flat face + low motion | neutral / resting |

This would help with the classes you noted as weak:

```text
fearful
disgust
angry
sad
```

Those are hard for a generic 7-class model.

---

## Recommendation 2: Keep discrete labels, but add explicit confidence / uncertainty

Your voting system already implicitly does this, but I would make confidence a first-class output.

For each face, the UI should understand:

```json
{
  "label": "happy",
  "confidence": 0.62,
  "margin": 0.18,
  "votes": 7,
  "quality_weighted": true,
  "calibrated_usual": false,
  "status": "clear"
}
```

or:

```json
{
  "label": null,
  "confidence": 0.24,
  "status": "uncertain",
  "flags": ["low_votes", "low_margin", "low_quality"]
}
```

That is better than only showing a dimmed “?”.

The UI can then distinguish:

```text
clear expression
moderate expression
uncertain expression
not enough evidence
```

That matches best practice.

---

## Recommendation 3: Consider valence/arousal as a secondary representation

A 7-class model is fine, but a more robust internal representation is:

```text
valence: negative → positive
arousal: low → high
```

You do not have to replace the 7-class model. You can add a mapping layer.

For example:

```text
happy       → high valence, moderate/high arousal
surprised   → neutral/high valence, high arousal
neutral     → neutral valence, low arousal
sad         → low valence, low/moderate arousal
angry       → low valence, high arousal
fearful     → low valence, high arousal
disgust     → low valence, moderate arousal
```

Then use valence/arousal for:

- smoother UI coloring
- per-person calibration
- anomaly detection
- disambiguation
- logging/debugging if ever allowed
- future multimodal fusion

For your current project, I would not over-engineer this. But it is worth adding as an internal debug layer.

---

## Recommendation 4: Centralize temporal expression state in one testable module

Right now the expression voting appears to be in `tracker_ui.html`. That is reasonable for a browser UI, but I would extract the logic into a small testable module.

Something like:

```js
ExpressionVoter
  - addScore(frame)
  - getState()
  - reset()
  - getConfidence()
  - getFlags()
```

and:

```js
UsualLookCalibrator
  - update(personId, label, confidence)
  - getUsual(personId)
  - forget(personId)
  - isTrusted(personId)
```

Benefits:

- easier unit testing
- easier reuse if the server later does the voting
- easier to change vote windows
- easier to add confidence flags
- less logic scattered across HTML

You can keep it browser-side, but make it modular.

---

## Recommendation 5: Decide whether expression voting should be server-side or browser-side

Your current design seems to be:

```text
server: per-frame expression score
browser: temporal voting + calibration + display
```

That is acceptable.

But there are trade-offs.

### Browser-side voting

Pros:

- keeps server simple
- user can adjust strictness locally
- per-tab behavior is isolated
- less server state

Cons:

- logic is in HTML/JS
- harder to test
- different tabs may diverge
- server cannot reason about expression over time

### Server-side voting

Pros:

- one source of truth
- easier testing
- can share state across clients
- can combine with recognition/quality state
- easier to add AUs/landmarks later

Cons:

- more server state
- need per-track IDs and expiry
- more complexity

For a personal local tool, browser-side is fine. But if you add richer expression features, I would move at least the temporal state into a shared Python or JS library.

---

## Recommendation 6: Make NPU/GPU selection evidence-based at runtime

Your hardware notes are excellent:

```text
GPU: 2.6–3.6 ms/face
CPU: ~12 ms/face
NPU: ~25 ms/face
```

That already suggests that, for your current model sizes and face counts, the GPU is likely the best default for recognition latency, while the NPU is more interesting for power/efficiency experiments.

I would add a runtime device report like:

```json
{
  "recognition": {
    "model": "arcface_w600k_r50",
    "device": "gpu",
    "provider": "DmlExecutionProvider",
    "latency_ms_per_face": 3.1
  },
  "quality": {
    "model": "ediffiqa_tiny",
    "device": "gpu",
    "provider": "DmlExecutionProvider",
    "latency_ms_per_face": 0.9
  },
  "expression": {
    "model": "mobilefacenet",
    "device": "gpu",
    "provider": "DmlExecutionProvider",
    "latency_ms_per_face": 0.7
  },
  "detection": {
    "model": "yunet",
    "device": "cpu"
  }
}
```

This would make it obvious when something silently falls back to CPU.

Important checks:

- Are ONNX sessions created with the intended providers?
- Does the expression model actually run on DirectML?
- Does the NPU path use the static batch-1 model?
- Does the system log which provider was selected?
- Does it warn if the requested provider is unavailable?
- Does it avoid NPU compile time during live use?
- Does it choose the fastest measured device by default?

For your current numbers, I would probably keep:

```text
default: GPU
fallback: CPU
optional: NPU benchmark / power-mode experiment
```

unless the NPU becomes significantly faster or more efficient in a later runtime.

---

## Recommendation 7: Add warmup and first-frame behavior

Since you have NPU compile time and ONNX session initialization, the first frame may be slower than steady-state.

I would add:

```text
model warmup at startup
first-frame latency report
provider report
optional “ready” flag before camera starts
```

For a local tool this is less critical, but it helps avoid confusion when the first few frames are slow.

---

## Recommendation 8: Add expression-specific regression checks

You already have:

```text
tools/expr_check.py
tools/eval_models.py
tools/hw_bench.py
```

That is excellent.

I would extend `expr_check.py` to report:

```text
per-class distribution
per-class confidence
mirror agreement by class
quality-bucket performance
head-turn-bucket performance
face-size performance
vote margin distribution
stale-drop rate
per-person usual-look drift
```

Especially useful buckets:

```text
quality: low / medium / high
head turn: front / medium / high
face size: small / medium / large
mirror agreement: high / low
```

This would tell you whether the model is failing because of:

- blur
- pose
- face size
- class ambiguity
- insufficient votes
- calibration drift

---

## Recommendation 9: Be careful with per-person calibration forgetting real negative states

Your “usual look” calibration is smart, but it has a subtle risk.

If someone’s usual look is calm/neutral, the system may downweight genuinely negative expressions as “usual” if the calibration is too slow or too broad.

Conversely, if someone usually looks serious, the system may need a stronger signal to detect happiness.

I would add safeguards:

```text
minimum calibration samples before trust
decaying confidence for stale calibration
separate calibration for valence vs arousal
do not suppress high-arousal deviations too aggressively
manual “Forget learned usual looks”
optional per-person sensitivity slider
```

Your existing “Forget learned usual looks” is good. I would just make the calibration behavior more explicit in the UI.

For example:

```text
Calibration: 34% learned
Usual look: neutral
Sensitivity: normal
```

or:

```text
Calibration: trusted after 100 frames
```

That would make the feature easier to trust.

---

## Recommendation 10: Add a simple “expression evidence” explanation

For each displayed expression, it would be useful to know why.

Example:

```text
happy: clear
evidence:
  - 8 of 10 votes happy
  - margin 21%
  - quality average 0.42
  - head turn 0.18
  - not usual
```

or:

```text
uncertain
evidence:
  - only 3 votes
  - margin 6%
  - quality low
  - head turn near limit
```

This does not need to be user-facing, but it would be extremely useful for debugging and tuning.

---

# 5. NPU/GPU-specific recommendations based on your docs

Your current setup:

```text
GPU: Radeon 8060S, ONNX Runtime DirectML
NPU: Ryzen AI 395, VitisAI / static ArcFace
CPU: fallback
```

and measured results:

```text
GPU: 2.6–3.6 ms/face
CPU: ~12 ms/face
NPU: ~25 ms/face
```

That suggests:

## Use GPU as default for interactive recognition

For a live webcam, 2.6–3.6 ms per face is excellent.

Even with multiple faces, quality scoring, and expression scoring, GPU should be comfortably real-time.

## Use NPU for experimentation, not necessarily default

At ~25 ms/face, NPU is still usable, but not clearly better than GPU for latency in your current measurements.

NPU may be more valuable for:

- battery/efficiency
- background processing
- avoiding GPU contention
- future larger models
- privacy/offload experiments

But for your current PoC, GPU seems like the best interactive default.

## Keep YuNet on CPU unless benchmark says otherwise

YuNet on CPU is simple and likely fast enough. Moving detection to NPU/GPU may not be worth the complexity unless detection becomes the bottleneck.

## Ensure the expression model’s device is explicitly known

You mention the expression model uses ONNX Runtime when available. I would verify:

```text
which provider runs facial_expression_recognition_mobilefacenet?
Is it DML?
Is it CPU?
Does it batch two mirrored runs?
Are the two mirrored runs sequential?
Is input preprocessing the dominant cost?
```

If mirrored expression scoring is ~0.7 ms/face on GPU, that is already small. But if it silently falls back to CPU, the overall frame latency may increase more than expected.

---

# 6. Architecture comparison with best practice

Your current expression path is probably close to:

```text
frame
  → YuNet detect
  → quality gate
  → head-turn gate
  → MobileFaceNet expression
  → mirror average
  → browser vote
  → calibrate
  → display
```

A stronger best-practice path would be:

```text
frame
  → face detect
  → track
  → quality / pose / size / motion gate
  → expression CNN
  → landmarks / blendshapes / AUs
  → temporal state
  → confidence / abstention
  → optional valence/arousal
  → per-person calibration
  → UI
```

Your system already has most of the important control logic. The main upgrade path is adding the richer facial-geometry channel and making confidence/evidence more explicit.

---

# 7. Documentation recommendation

You have:

```text
AGENTS.md
CLAUDE.md
PROJECT_CONTEXT.md
README.md
```

`AGENTS.md` and `CLAUDE.md` appear to be duplicate content.

I would avoid maintaining two near-identical agent instruction files manually. Options:

1. Make `AGENTS.md` the canonical file and have `CLAUDE.md` reference it.
2. Generate one from the other.
3. Keep only one, depending on which tools you use.
4. Use a small build step to copy/symlink.

Otherwise they may drift.

`PROJECT_CONTEXT.md` is excellent. It is the most valuable file for an assistant because it captures:

- owner preferences
- measured thresholds
- environment gotchas
- data model
- history
- vocabulary
- hardware behavior

That is much better than a generic README.

---

# 8. What I would inspect next in the code

Now that the high-level docs are clear, the most useful files to review are:

## Priority 1

```text
face-recognizer/face_id.py
```

Focus on:

```text
Faces.expression()
Faces._optional()
head_turn()
ONNX session creation
FACE_DEVICE handling
provider selection
quality gating
expression gating
mirrored scoring
model loading
device reporting
```

---

## Priority 2

```text
face-recognizer/tracker.py
```

Focus on:

```text
Engine.recognize()
per-face result construction
lock handling
people.npz reload
strictness handling
expression gating
response format
error handling
device/model startup logs
```

---

## Priority 3

```text
face-recognizer/tracker_ui.html
```

Focus on the expression section:

```text
// ---- expression
X_MIN_VOTES
X_SURE
X_MARGIN
vote windows
quality weighting
calibration / localStorage
usual look logic
stale-drop logic
UI rendering
confidence display
```

---

## Priority 4

```text
face-recognizer/tools/expr_check.py
face-recognizer/tools/hw_bench.py
face-recognizer/tools/eval_models.py
```

Focus on:

```text
what is measured
how latency is measured
how NPU/GPU/CPU are compared
how expression confidence is reported
how mirror agreement is calculated
how thresholds are validated
```

---

## Priority 5

```text
face-recognizer/requirements.txt
face-recognizer/models/npu/vitisai_config.json
face-recognizer/README.md
```

Focus on:

```text
onnxruntime-directml version
NPU runtime dependencies
model licenses
README wording
user-facing expression claims
```

---

# 9. Initial recommendation summary

## Keep

- local-only design
- separate labeling tool and tracker
- measured thresholds
- quality/head-turn gating
- mirrored scoring
- temporal voting
- per-person calibration
- honest expression wording
- NPU/GPU benchmarking

## Improve

- add facial geometry / landmarks / AUs as a secondary expression channel
- make confidence and uncertainty explicit
- extract expression voting/calibration into a testable module
- add expression regression reports by quality/pose/face size
- add runtime device/provider reporting
- avoid duplicate agent docs drifting
- consider internal valence/arousal representation for smoother behavior
- make calibration trust and sensitivity visible

## Do not do casually

- do not change measured thresholds without re-measuring
- do not claim emotion as internal state
- do not store expression data unless you explicitly decide to
- do not let NPU compile happen in the live path
- do not silently fall back to CPU without logging
- do not over-trust rare classes: fearful, disgust, sad, angry

---

# 10. My current judgment

Your PoC is already architecturally sound and unusually careful for this kind of system.

The biggest opportunity is not “add a bigger emotion model.” The bigger opportunity is to make the expression signal more interpretable and more robust by combining:

```text
current 7-class model
+ face geometry / AUs / blendshapes
+ explicit confidence
+ better temporal state
+ better regression reporting
```

That would move it from:

```text
a working expression classifier
```

toward:

```text
a robust real-time facial-expression estimator
```

without turning it into an overconfident “emotion detector.”

If you want, next I can review the actual code in this order:

```text
1. face_id.py — expression + device/session logic
2. tracker.py — server gating and response
3. tracker_ui.html — voting/calibration/UI
4. tools/expr_check.py + hw_bench.py — validation
```

Paste those files whenever you’re ready, and I’ll give file-level recommendations.
