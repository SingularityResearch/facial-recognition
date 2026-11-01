# Why the expression reading is layered

Why the live tracker reads expression the way it does, and the research behind each layer.

**Related docs:**
- [Project context](../PROJECT_CONTEXT.md): background, history and the measured tuning values.
- [Face Recognizer README](../face-recognizer/README.md): user docs, including the **Expression** section.
- [Real-Time Facial Emotion Recognition](../Real-Time%20Facial%20Emotion%20Recognition.md): the design review that led to the 2026-10-30 expression refactor.
- [Face Tracker README](../face-tracker/README.md): the original standalone browser tracker.
- [CLAUDE.md](../CLAUDE.md) / [AGENTS.md](../AGENTS.md): conventions for coding assistants.

## Summary

A camera cannot read feelings. It can only measure what a face or object looks like and how it moves.
Those measurements are compared over time and against the subject's own normal. Each layer below fixes one
way that single-frame "emotion detection" fails. The most the system can honestly produce is a hypothesis
about a hidden state, with its evidence attached. That is why the tracker says a face **looks** happy,
never that the person **feels** happy.

## The core problem: feelings are hidden

Science treats a feeling as a **latent state**. You only see its outputs: facial muscle movement, head and
body motion, voice and behavior. Two findings shape the whole design:

- **The link from face to feeling is weak and depends on context.** Barrett et al.'s 2019 review,
  "Emotional Expressions Reconsidered", found that people make the "textbook" expression for an emotion
  well under half the time they report feeling it. The same expression also turns up with different
  feelings. The basic-emotion camp (Ekman) and the constructionist camp (Barrett) disagree on a lot, but
  they agree the mapping is many-to-many.
- **Context can flip the reading.** In Aviezer, Trope & Todorov (2012, *Science*), people couldn't tell
  tennis players' winning faces from their losing faces when they saw the face alone. With the body
  included, they read them correctly, and they gave the face the credit.

So any single cue is a noisy, biased measurement of the hidden state. The defensible approach is several
independent measurements, each checked for quality, combined over time and compared to a baseline.

## The layers

| Layer | In this project | Why it's there |
|---|---|---|
| 1. Detect and track | YuNet + per-track identity ([tracker.py](../face-recognizer/tracker.py)) | Emotion is a process over time, so you need the same subject across frames to build up evidence. Tracking also gives **kinematics** (speed, acceleration, approach or retreat) for any object, not only faces. |
| 2. Quality gate | `Q_EXPR` 0.25 and `MAX_TURN` about 35° ([face_id.py](../face-recognizer/face_id.py)) | Blurry or turned faces make the model *confidently wrong* (usually "sad" or "neutral"), not unsure. In signal-detection terms, abstaining beats emitting noise that looks like signal. |
| 3. Appearance | 7-class expression model + MediaPipe blendshape cues | Blendshapes are close to **FACS action units** (Ekman & Friesen 1978): anatomical muscle movements that are fairly objective. Labels like "happy" are interpretations on top of them. The two are kept separate, so cues are evidence only. |
| 4. Time voting | Last 10 scored frames, at least 3 votes, at least 45% share and a lead (`X_MIN_VOTES`, `X_SURE`, `X_MARGIN` in [expression.js](../face-recognizer/expression.js)) | A real expression has an onset, a peak and a fade, typically over 0.5 to 4 s. Collecting votes over time is evidence accumulation (the drift-diffusion idea): commit once the evidence crosses a threshold. |
| 5. Personal baseline | Each person's "usual look", learned in the browser | Resting faces differ: one person's neutral reads as "sad" or "angry" to a model. A **departure from someone's own baseline** says more than an absolute score. Psychophysiology baselines for this reason, and it also cancels some face-specific model bias. |
| 6. Dimensions | Rough valence and arousal in the evidence panel | Russell's circumplex (1980): pleasant↔unpleasant × calm↔activated holds up better than 7 categories. Arousal also tracks motion energy, which links layers 1 and 3. |
| 7. Fusion | Measured before it is used ([tools/expr_eval.py](../face-recognizer/tools/expr_eval.py)) | See the next section. |
| 8. Prediction | Short horizon only | Predicting where something moves next is physics (Kalman-style filtering) and works well. Predicting intent relies on theory of mind and is far weaker. |

## Why fusing several cues works, and when it doesn't

Combining n independent noisy estimates cuts the error by roughly √n. The key word is **independent**. If
the expression model and the blendshape cues are fooled by the same thing (lighting, a resting frown),
fusing them doubles the confidence without adding any information. That is why the project rule says
face-shape cues stay evidence only until fusion is measured on posed sessions. The math rewards fusion only
when the errors are uncorrelated, and that has to be measured, not assumed.

These cues fail in different ways, so they are independent enough to fuse:

- **Appearance:** face shape and muscle cues.
- **Motion:** head speed, stillness, approach and retreat.
- **Context:** who else is present, the time, the setting.
- **Personal history:** the person's baseline.

## The same layers work for other objects

Heider & Simmel (1944) showed people triangles and circles moving on a screen. Viewers described them as
bullying, fleeing and afraid, from **motion alone**. Johansson's point-light displays (1973) showed that a
dozen moving dots are enough for people to see a walker's mood and intent. So the layers carry over to
anything that can be tracked:

track → kinematics → state features → per-subject baseline → context → short-horizon prediction

Animals already have coding systems like FACS (DogFACS, EquiFACS). For vehicles or robots, the "affect" is
patterns of behavior such as hesitation, aggression or erratic motion.

## On "unbiased"

No pipeline is unbiased. Bias comes in from three places:

- **Training data:** expression datasets are mostly posed and mostly Western.
- **Labelers:** they disagree with each other, and their culture shapes the labels.
- **Faces:** Buolamwini & Gebru (2018, "Gender Shades") showed that error rates vary with skin tone and
  gender.

The design limits bias but cannot remove it. It does so in four ways:

- It compares each person to their own baseline.
- It abstains on poor frames.
- It shows the evidence instead of a bare verdict.
- It measures error **per person** on local posed sessions.

## How far "what it might be feeling" can go

The best the tracker can say is something like:

> *Looks surprised, which is unusual for this person. Motion shows high arousal, and 3 of 3 cues agree.
> Confidence: moderate.*

That is a hypothesis. Turning "looks" into "feels" needs ground truth, and only **self-report** provides
it, for example a prompt asking "how do you feel right now, 1 to 7?". Without self-report, accuracy can be
measured against *posed* expressions but never against feelings. That limit is why the
[README](../face-recognizer/README.md) says "looks" and not "feels": it is the limit of what the data can
show.

The EU AI Act bans emotion recognition in workplaces and schools. That doesn't affect personal, local use
like this project, but it matters if the project is ever used elsewhere.

## Next experiments

1. **Measure fusion.** Use posed sessions in `tools/expr_eval.py` to check whether the cues' errors are
   uncorrelated before combining them.
2. **Add motion as an arousal signal.** Use head speed and motion energy from the tracker. These don't
   depend on appearance.
3. **Add opt-in self-report.** It is the only way to get "feels" ground truth, even with a few dozen
   samples per person. Any self-report would need to follow the rule that expression is never stored
   outside opt-in test sessions.

## References

- Aviezer, H., Trope, Y., & Todorov, A. (2012). Body cues, not facial expressions, discriminate between
  intense positive and negative emotions. *Science*, 338(6111), 1225–1229.
- Barrett, L. F., Adolphs, R., Marsella, S., Martinez, A. M., & Pollak, S. D. (2019). Emotional expressions
  reconsidered: Challenges to inferring emotion from human facial movements. *Psychological Science in the
  Public Interest*, 20(1), 1–68.
- Buolamwini, J., & Gebru, T. (2018). Gender Shades: Intersectional accuracy disparities in commercial gender
  classification. *Proceedings of Machine Learning Research*, 81, 77–91.
- Ekman, P., & Friesen, W. V. (1978). *Facial Action Coding System*. Consulting Psychologists Press.
- Heider, F., & Simmel, M. (1944). An experimental study of apparent behavior. *The American Journal of
  Psychology*, 57(2), 243–259.
- Johansson, G. (1973). Visual perception of biological motion and a model for its analysis. *Perception &
  Psychophysics*, 14(2), 201–211.
- Russell, J. A. (1980). A circumplex model of affect. *Journal of Personality and Social Psychology*, 39(6),
  1161–1178.
