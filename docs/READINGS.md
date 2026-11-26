# What each reading means

Face Recognizer reads a lot from each camera, microphone and device. This page explains every one of those readings
(the app's sensors), one at a time:

- what it is;
- what its values mean: what is low, usual and high;
- why it matters: what a change in it can point to;
- what the app does with it;
- where you see it.

The app carries this page: on the Face tracking and Cameras tabs, **What each reading means** opens it, and so does
a reading's name in the Signals chart or in a camera's readout, at that reading. Where a reading has another name
somewhere in the app, that name is given too. The **key** is the name the reading has
in what the app keeps (the signal history, an alert's details) and in Analyze.

Every reading here is evidence of how a face **looks**, how a voice **sounds** or how a body **moves**. None of them
says what a person feels or thinks, and none means the same for everyone: that is why the app learns each person's
own usual. The body readings (pulse, breathing) are estimates from a camera. They are not a medical device and do not
diagnose, treat, cure or prevent any medical condition. Ask a healthcare professional about anything to do with your
health.

---

## Words used on this page

- **Check**: one look at the camera's picture. The app checks the picture several times a second, and every check
  gives a new value for most readings.
- **Score from 0 to 1**: many readings are a number between 0 (none at all) and 1 (as much as there can be). A smile
  of 0.8 is a broad smile; 0.1 is hardly one.
- **Share** (%): how much of something there was. A gaze-away share of 30% means the eyes looked away in 30% of
  the checks.
- **Per minute** (/min) and **per second** (/s): how often something happens, such as blinks, heartbeats or
  syllables.
- **Hz** (hertz): how many times a second something repeats. A voice's pitch of 120 Hz means the vocal cords open and
  close 120 times a second. A higher number is a higher sound.
- **Semitone** (st): one step on a piano keyboard. Twelve semitones make an octave.
- **dB** (decibel): how much louder one sound is than another. +10 dB sounds about twice as loud.
- **dBFS** (decibels below full scale): how loud a sound is compared with the loudest a device can record or play.
  0 dBFS is that loudest, so the numbers are negative, and a number nearer 0 is louder. −20 is loud, −50 is quiet,
  and below −65 is near silence.
- **Valence**: how pleasant or unpleasant something is. A smile looks pleasant (positive valence). A frown, or a look
  of disgust, looks unpleasant (negative valence).
- **Arousal**: how calm, or how energetic and worked up, something is. Sleepy, relaxed or bored is low arousal;
  excited, surprised, angry or afraid is high. Arousal says nothing about pleasant or unpleasant: joy and fear are
  both high arousal, and contentment and sadness both low.
- **Dominance**: how forceful and in control a voice sounds. A confident command is high dominance; a hesitant,
  pleading or timid voice is low.
- **Face-shape cue**: how much one part of the face is moving in one way, such as the corners of the mouth pulled up
  or the brows drawn down. A model of the face (MediaPipe's face landmarker) gives each cue a score from 0 to 1 at
  every check.
- **Body points**: where the nose, ears, shoulders, elbows, wrists, fingertips and hips are in the picture, found by a
  model of the body (MediaPipe's pose landmarker).
- **Shoulder width** and **face width**: the body readings are measured in shoulder widths and the face readings in
  face widths. So they read the same whether a person is near the camera or far from it.
- **Their usual** (a person's normal): a person's own average for a reading, learned over time. The same number can
  be usual for one person and unusual for another.
- **Spread** (SD, the standard deviation): how much a reading normally goes up and down for a person. A reading
  within about 2 spreads of their average is usual for them.
- **Spreads away** (z): how far a reading is from their usual, counted in spreads. 0 is their average; +1 is a little
  above; −2 or +2 or further counts as unusual.
- **Learning**: not enough of this person has been seen yet to know their usual.

---

## This device

What the device that runs the app measures about the room and about itself: who is in view, its camera, its own
processor and battery, and the sound it hears and plays.

### People in view

#### In view

- **What it is**: a timeline of who the camera saw. There is one lane for each named person and one for faces nobody
  is named for.
- **Its values**: a lane is marked whenever that person was seen. Up to 8 lanes; a gap of more than 2.5 seconds
  breaks the line.
- **Why it matters**: it shows at a glance who was there, when, and for how long, and lines up every other reading
  with the people it belongs to.
- **What the app does with it**: marks "Ann in view" in the chart's events when someone appears, and "Ann left" after
  10 seconds without them.
- **Where you see it**: the Signals chart, People in view, "in view".

#### Faces in view

- **What it is**: how many faces the camera's picture holds at the latest check.
- **Its values**: a count: 0, 1, 2 and so on. The history also keeps how many of them were unknown (key `unknown`).
- **Why it matters**: people look, sound and sit differently alone and in company, and a face nobody is named for may
  be someone new.
- **What the app does with it**: one face in view counts as "alone" for a person's usual (see
  [Alone or with others](#alone-or-with-others)).
- **Where you see it**: the Signals chart, People in view; alerts, "Faces in view". Key: `faces`.

### Camera

#### Checks

- **What it is**: how many times in the last second this device looked at its camera's picture for faces.
- **Its values**: checks a second. About 1 on a Raspberry Pi, a few on a phone, and 20 to 60 on a fast computer.
- **Why it matters**: every reading of a person is taken at a check, and nothing is read between two of them. Under
  about 5 a second, quick things are missed (a blink, a passing look), and the pulse and expression are less sure.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Camera, "checks". Key: `checks`.

#### One check

- **What it is**: how long one check took. That is from sending the picture to having every answer back: the faces
  found, who they are, how sharp they are, their expression, and the face, body and hand points.
- **Its values**: milliseconds (thousandths of a second). About 10 to 40 on a fast computer; several hundred on a
  slow device.
- **Why it matters**: it is how far the readings lag behind what the camera sees, and a long check means fewer checks
  a second.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Camera, "one check". Key: `check`.

The Camera section's text also says what the camera gives, such as "1280×720 at 30 pictures a second".

### Device

What the device says about itself, asked every 3 seconds. A reading that the device's own system doesn't give has no
chart.

#### Processor in use

- **What it is**: how busy the device's processor (CPU) is, with every program together.
- **Its values**: 0% (idle) to 100% (fully busy).
- **Why it matters**: near 100%, the device can't keep up: checks slow down and the readings get coarser.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "processor in use". Key: `load`.

#### Memory in use

- **What it is**: how much of the device's memory (RAM) is in use, by every program together.
- **Its values**: 0% to 100%.
- **Why it matters**: near 100%, everything on the device slows down, and a phone may close the app.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "memory in use". Key: `memory`.

#### Temperature

- **What it is**: the warmest of the device's own temperature sensors.
- **Its values**: degrees Celsius. A processor at work is often 50 to 75 °C; 80 °C or more is hot. Only Linux
  computers and the Raspberry Pi give one; Windows and a Mac don't give the app a temperature.
- **Why it matters**: a device that gets too hot slows itself down to cool off, and the checks slow with it.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "temperature". Key: `temp`.

#### Heat state

- **What it is**: a phone's own word for how hot it is. A phone gives no degrees.
- **Its values**: nominal (normal), fair (warm), serious (hot: the phone slows itself down), critical (too hot).
- **Why it matters**: from serious on, the phone slows down and the checks slow with it.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "heat state" (phones only). Key: `heat`.

#### Battery

- **What it is**: how full the battery is, on a device that has one.
- **Its values**: 0% (empty) to 100% (full).
- **Why it matters**: a device running on its battery stops when it is empty, and its camera with it.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "battery". Key: `battery`.

#### On power

- **What it is**: whether the device is plugged in.
- **Its values**: yes or no. A full battery that is plugged in says yes, even though it isn't charging any more.
- **Why it matters**: a device on power can run for as long as you like.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Device, "on power". Key: `charging`.

The Device section's text also says what the device is: its kind and model, its system, the app's version, its
processor and memory, and where each model runs.

### Sound: speakers and microphones

These readings are taken only while **Chart this device's sound** is on. The app listens to each of the device's
sound interfaces for 4 seconds at a time while the page is open, and makes one reading of each 4 seconds:

- **speakers**: what the device itself plays;
- **microphone**: what it hears in the room.

The sound is measured in memory and never kept: only these numbers and one word are. Together they say what was going
on around the people in view: a quiet room, a conversation, music, a television.

#### Heard

- **What it is**: what kind of sound it is, in one word, as a sound-recognition model (YAMNet) judges it.
- **Its values**: quiet, music, speech, television or radio, steady background noise, activity, alert sounds, animal
  sounds, or other sounds.
  - Music or speech is chosen when it makes up at least a quarter of what was heard.
  - Quiet is chosen when the level is below −65 dBFS.
  - Otherwise the largest kind is chosen.
  - With the model missing, "other sounds" also stands for sound that comes and goes.
- **How sure** (key `sure`, kept in the history and shown in the music log): the model's score for its word, from
  0 to 1. Above 0.5 is fairly sure; under 0.2 is a guess.
- **Why it matters**: it is the room's context in one word, and the music log is built on it.
- **What the app does with it**: shows it, and marks "music heard" in the chart's events when music starts. It is a
  guess about the sound, not a fact about the room.
- **Where you see it**: the Signals chart, Sound, "heard". Key: `what`.

#### Level

- **What it is**: how loud the sound is overall.
- **Its values**: dBFS, from 0 (the loudest the device can handle) down. −20 is loud, about −50 is a quiet room, and
  below −65 counts as quiet.
- **Why it matters**: it says how loud the room or the music is, and a loud room makes the voice readings harder.
- **What the app does with it**: shows it, and uses it to decide "quiet".
- **Where you see it**: the Signals chart, Sound, "level". Key: `level`.

#### Steady background

- **What it is**: the level the sound rarely goes below: the room's or the music's steady background.
- **Its values**: dBFS, as for Level. A quiet room is about −50 or lower.
- **Why it matters**: a high background (a fan, traffic, a crowd) hides quieter sounds and voices.
- **What the app does with it**: keeps it in the history.
- **Where you see it**: the signal history and Analyze only. Key: `floor`.

#### Events

- **What it is**: how far the loud moments stand out above the steady background (microphone only).
- **Its values**: dB. Under about 6 is an even sound; 15 or more means things happening, such as talk, a door or a
  dog.
- **Why it matters**: it tells an active room from a still one.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "events". Key: `events`.

#### Steady

- **What it is**: how even the loudness is (microphone only).
- **Its values**: 0 to 1. Near 1, the sound hardly changes (a fan, a hum). Near 0, it comes and goes (talk, music,
  activity).
- **Why it matters**: it tells a machine's noise from people and things happening.
- **What the app does with it**: shows it, and uses it to tell steady background noise apart from activity.
- **Where you see it**: the Signals chart, Sound, "steady". Key: `steady`.

#### Speech-like

- **What it is**: how much the sound rises and falls at the pace of syllables (two to eight a second) in the range of
  a voice, as speech does (microphone only).
- **Its values**: 0 to 1. A quiet room or steady noise often reads about 0.4 to 0.6 too, so a single value says
  little; a clear rise above the room's own level is what points to talk.
- **Why it matters**: it says whether someone is talking nearby, even when the model's word is something else.
- **What the app does with it**: shows it. It doesn't decide the word in Heard.
- **Where you see it**: the Signals chart, Sound, "speech-like". Key: `speechlike`.

#### Mains hum

- **What it is**: how strongly the hum of the electricity supply is heard (microphone only). That hum is 50 or 60 Hz,
  depending on the country.
- **Its values**: dB above the sound around it. Under 5 is none to speak of; 10 or more is a clear hum.
- **Why it matters**: a hum comes from something electrical near the microphone (a fan, a transformer, a cable), and
  it gets into every voice reading.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "mains hum". Key: `hum`.

#### Hiss

- **What it is**: how much the sound is like hiss or rushing air rather than tones.
- **Its values**: 0 to 1. Near 1 is hiss-like noise (air, rain, a fan); near 0 is clear tones, such as a note or a
  whistle.
- **Why it matters**: it helps tell noise from music and voices in the history.
- **What the app does with it**: keeps it in the history.
- **Where you see it**: the signal history and Analyze only. Key: `flat`.

#### Tempo

- **What it is**: the speed of the beat of the music the device plays (speakers, while it is music).
- **Its values**: beats a minute, from 50 to 200. A slow ballad is about 60 to 80; pop about 100 to 130; dance music
  120 to 130 and up. Where the beat could be read at half or double speed, the app picks the one people usually tap
  along to.
- **Why it matters**: it describes the music that was playing, for the music log and for comparing with how people
  looked and moved meanwhile.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "tempo". Key: `tempo`.

#### Beat clarity

- **What it is**: how clear and regular the music's beat is (speakers, while it is music).
- **Its values**: 0 to 1. Higher is a stronger, steadier beat, as in dance music; lower is a loose or hidden one, as
  in much classical or ambient music.
- **Why it matters**: it describes the music's character, and says how far to trust the tempo.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "beat clarity". Key: `pulse` (in the Sound section; the Face
  section's pulse is the heartbeat).

#### Brightness

- **What it is**: whether the music's sound leans to the deep end or the treble end (speakers, while it is music).
- **Its values**: Hz: the middle of the sound's weight. Most music lies between about 500 and 4,000; lower is darker
  and warmer, higher is brighter and sharper.
- **Why it matters**: it describes the music's character.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "brightness". Key: `bright`.

#### Bass share

- **What it is**: how much of the music's energy is deep bass, below 150 Hz (speakers, while it is music).
- **Its values**: 0 to 1. Higher is more bass-heavy: dance music and hip-hop read higher than speech or acoustic
  music.
- **Why it matters**: it describes the music's character.
- **What the app does with it**: shows it.
- **Where you see it**: the Signals chart, Sound, "bass share". Key: `bass`.

---

## One person

The readings below are for each face in view, check by check. A face nobody is named for has them too, but no usual
of its own.

### Recognition

Who the face is, and how far to trust the name.

#### Match

- **What it is**: how alike this face is to the person it was named as.
  - The app turns each face into a print: 128 numbers that describe the face, made by the app's own recognition
    model, Opsis 1.
  - It compares that print with the prints it has stored of each person.
  - A person's score is the average of their 5 closest prints, so one odd photo doesn't decide it.
- **Its values**: 0% to 100%. Higher is more alike. A named face usually reads about 65% to 90%. A match just above
  the [strictness](#strictness) is a close call; one far above it is a sure name.
- **Why it matters**: it is how sure the name is. A match that stays low for someone you know means their prints
  don't cover how they look now (other light, glasses, a new haircut): name a few more of their faces.
- **What the app does with it**: names the face as the best-matching person when the match reaches the strictness;
  otherwise the face is "Unknown".
- **Where you see it**: under the face; the Signals chart, Recognition, "match"; alerts, "Match to their face prints".
  Key: `match`.

#### Strictness

- **What it is**: how alike a face must be to a person's prints to be named as them. It is a setting, not a reading
  (**Match strictness**, in Face tracking's Recognition card).
- **Its values**: 25% to 75%. Higher gives fewer wrong names but more faces left Unknown; lower names more faces but
  makes more mistakes. The default suits the recognition model.
- **Why it matters**: it is the trade between a wrong name and no name.
- **What the app does with it**: compares every match with it.
- **Where you see it**: the dashed line on the match chart.

#### Named from averaged prints

- **What it is**: how the name was decided.
  - The app keeps a face's last 7 prints, each counted by how sharp its picture was.
  - Once it has at least 3, it names the face from their average, which a single blurry frame can't sway.
  - Before that, each of the last frames votes for a name, and a sharp, close match counts for more.
- **Its values**: yes (from the averaged prints) or no (from the frames' votes).
- **Why it matters**: a name from averaged prints is steadier and more sure than one from a few frames.
- **What the app does with it**: the unusual alert is raised only for a face named this way.
- **Where you see it**: alerts, "Named from averaged prints". Key: `namedFromAveragedPrints`.

#### Picture quality

- **What it is**: how usable the picture of the face is for recognizing it, as a face-quality model judges it. It
  takes in sharpness, lighting and how far the face is turned.
- **Its values**: 0 to 1. Under 0.25 is poor (blurry, dark or turned away); 0.3 to 0.5 is fair, and most webcam
  pictures lie there; above 0.5 is good; 0.7 or more is excellent.
- **Why it matters**: everything read from the face (who it is, the expression, the cues) is only as good as its
  picture. Low quality most often means poor light, movement or a face turned away.
- **What the app does with it**: a sharp picture counts for more than a blurry one everywhere. Only pictures of at
  least 0.30 are used to learn a person's face, and the expression isn't read below 0.25.
- **Where you see it**: the Signals chart, Recognition, "picture quality"; alerts, "Face picture quality". Key:
  `quality`.

#### Face size

- **What it is**: how wide the face is in the camera's picture.
- **Its values**: pixels. Under 40 is too small to recognize; 100 or more is comfortable.
- **Why it matters**: a small face (far away, or a wide camera) has less detail, so every reading of it is less sure.
- **What the app does with it**: leaves out faces under 40 pixels.
- **Where you see it**: the Signals chart, Recognition, "face size"; alerts, "Face size". Key: `size`.

### Expression

How the face looks. A model of facial expression gives every check a score for each of seven looks: angry,
disgust, fearful, happy, neutral, sad and surprised. A check is read only when the face is sharp enough and not
turned too far away, because a blurry or turned face makes the model wrong rather than unsure. The label is made from
the last checks together, so one frame can't change it. It is a look, never a feeling: people smile when they are not
happy, and look neutral when they are.

#### The label

- **What it is**: the look the recent checks agree on, shown as a word under the face.
- **Its values**:
  - **happy**: a clear reading;
  - **happy?**: a reading that isn't clear (see [Why it isn't clear](#why-it-isnt-clear));
  - **neutral · usual**: a clear reading that is this person's usual look;
  - a word in the highlight colour: a change from this person's usual look, shown while **Calibrate to each person**
    is on. It is the look that stands out most against how this person usually looks.
  - "no expression yet": not enough checks have been read.
- **Why it matters**: it sums up how the face looks right now in one word. A change from a person's usual look says
  more than the word alone.
- **What the app does with it**: shows it, keeps it in the expression log, and puts it in an alert's details. No
  expression raises an alert by itself.
- **Where you see it**: under the face; alerts, "Label". Key: `label`.

#### Votes

- **What it is**: how many of the latest checks the label is made from. Each check that was read is one vote.
- **Its values**: 0 to 10 (about the last second). A sharp picture's vote counts for more than a blurry one's. A label
  needs at least 3 votes to be clear.
- **Why it matters**: few votes mean the face was mostly blurry, turned away or just arrived, so the label rests on
  little.
- **What the app does with it**: decides with it whether the label is clear.
- **Where you see it**: the expression evidence under the face; alerts, "Votes (frames)". Key: `votes`.

#### Share of votes

- **What it is**: how much of the votes went to the label's look. It is that look's average score over the votes,
  with sharper pictures counting for more.
- **Its values**: 0% to 100%. 62% means the checks gave that look 62% of their weight, and the other six looks shared
  the rest. Under 45% the label isn't clear; a clear label is usually 70% or more.
- **Why it matters**: it is how strongly the face shows that look.
- **What the app does with it**: decides with it whether the label is clear.
- **Where you see it**: the expression evidence; the Signals chart, Expression, "share of votes"; alerts, "Share of the
  votes". Key: `share`.

#### Lead

- **What it is**: how far the label's look is ahead of the next one: its share minus the runner-up's share.
- **Its values**: 0% to 100%. A small lead means two looks are nearly tied, such as neutral 40% against sad 35%, a
  lead of 5%. Under 12% the label isn't clear; above 30% there is no doubt between the two. For a change from a
  person's usual look, the lead is measured against their usual.
- **Why it matters**: a face between two looks (half neutral, half sad) is mixed, and the label says only the larger
  half.
- **What the app does with it**: decides with it whether the label is clear.
- **Where you see it**: the expression evidence; alerts, "Lead over the next label". Key: `lead`.

#### Why it isn't clear

- **What it is**: why the label carries a question mark, or why the latest check wasn't read.
- **Its values**:
  - few votes, low share, low lead: the label missed one of the three conditions above;
  - blurry: the face's picture was too poor to read;
  - turned: the face was turned too far away;
  - talking or hand: the check was skipped because the person was talking or had a hand on the face (only with the
    options that skip those switched on);
  - learning: their usual look is still being learned.
- **Why it matters**: it says whether to wait for a better view or take the label as it is.
- **What the app does with it**: shows it.
- **Where you see it**: the expression evidence; alerts, "Why it isn't clear". Key: `flags`.

#### Share of each look

- **What it is**: the latest check's score for each of the seven looks.
- **Its values**: seven shares that add up to 100%. It moves frame by frame, unsmoothed.
- **Why it matters**: it shows the mix behind the label, and brief looks too quick for the label.
- **What the app does with it**: shows it as the coloured band of the Expression chart, and keeps each look's average
  in the history.
- **Where you see it**: the Signals chart, Expression, "share of each look". Key: `looks` in the history.

#### Expression shares

- **What it is**: each look's average score over the last 30 seconds.
- **Its values**: seven shares that add up to 100%.
- **Why it matters**: it shows how the face looked in the time before an alert, not just at its moment.
- **What the app does with it**: puts them in an alert's details.
- **Where you see it**: alerts.

#### Held

- **What it is**: how long the current clear label has been shown.
- **Its values**: seconds.
- **Why it matters**: a look held for a long time is a steady state; a flash of a look is a passing one.
- **What the app does with it**: shows it.
- **Where you see it**: the face evidence under the face.

#### Their usual look

- **What it is**: how this person usually looks: their own average of the seven looks. It is learned while
  **Calibrate to each person** is on, from their sharp pictures, and trusted after 100 of them.
- **Its values**: "usual look 40% learned" says how far the learning has come.
- **Why it matters**: faces differ at rest: some look a little sad or stern when they are not. Measured against their
  own usual, a change shows what is new for that person.
- **What the app does with it**: the label measures a change against it. It is kept on the device and shared with
  your other devices. **Forget what's learned** clears it.
- **Where you see it**: the expression evidence; alerts, "Their usual look learned". Key: `usualLookLearned`.

#### Valence (face)

- **What it is**: how pleasant or unpleasant the face looks, in one number. Each of the seven looks has a textbook
  place, and the reading is the average of those places, weighted by each look's share:
  - happy is clearly pleasant, and surprised a little;
  - neutral is in the middle;
  - sad, angry, fearful and disgust are unpleasant.
- **Its values**: −1 (very unpleasant) through 0 (neither) to +1 (very pleasant). Within ±0.2 is about neutral, where
  most faces are most of the time.
  Marked **fit** where the app has a fit made from your own feeling reports and uses it instead.
- **Why it matters**: it puts the seven looks on one line, which is easier to follow over time and in a chart than
  seven shares.
- **What the app does with it**: shows it. It never changes the label.
- **Where you see it**: the expression evidence ("V+.40"); the Signals chart, Expression, "valence"; alerts, "Valence".
  Key: `valence`.

#### Arousal (face)

- **What it is**: how calm or how energetic the face looks, made the same way from the seven looks' places:
  - surprised, fearful and angry are high;
  - happy and disgust are a little above the middle;
  - neutral is in the middle;
  - sad is low.
- **Its values**: −1 (very calm or low) through 0 to +1 (very excited or worked up). Marked **fit** as for valence.
- **Why it matters**: with valence it places the look on the usual map of looks: pleasant and lively (happy),
  unpleasant and lively (angry, afraid), unpleasant and low (sad).
- **What the app does with it**: shows it. It never changes the label.
- **Where you see it**: the expression evidence ("A+.20"); the Signals chart, Expression, "arousal"; alerts, "Arousal".
  Key: `arousal`.

#### Calibrated confidence

- **What it is**: how often a reading as sure as this one has been right before. It is known only where the app has a
  calibration, made from earlier sessions in which the looks were posed.
- **Its values**: 0% to 100%. "64%" means readings like this one were right about 64 times in 100. Missing where there
  is no calibration.
- **Why it matters**: the model's own share can be too sure or not sure enough; this says how far to trust it.
- **What the app does with it**: shows it.
- **Where you see it**: the expression evidence; alerts, "Calibrated confidence". Key: `calibratedConfidence`.

#### State model

- **What it is**: a second way of reading the expression, switched on with **Use state model for expression** (off
  unless you switch it on). The label counts the last votes; the state model carries its belief from check to check
  and changes it only as the new checks say.
- **Its values**: its own look and share ("State model says").
- **Why it matters**: it is steadier through a blurry moment, but slower to follow a real change. It is still being
  tried out.
- **What the app does with it**: shows its look in the evidence when it disagrees with the label.
- **Where you see it**: the expression evidence; alerts, "State model says" and "State model share". Key:
  `stateModelSays`.

#### Cues for this look

- **What it is**: the [face-shape cues](#the-other-face-shape-cues) that go with the label's look:
  - happy: smile and cheek raise;
  - sad: inner brow up and frown;
  - surprised: outer brow up, eyes wide and jaw open;
  - fearful: inner brow up, eyes wide and stretch;
  - angry: brow down, eye squint and press;
  - disgust: sneer and upper lip.
- **Its values**: each cue's score, 0 to 1.
- **Why it matters**: they show whether the face's own movements back the label. A happy label with no cheek raise,
  for instance, is a smile of the mouth alone.
- **What the app does with it**: shows them as evidence beside the label. They never change it.
- **Where you see it**: the expression evidence.

### Face

Read from the face itself: its movement, its eyes and mouth, its shape and its skin. Most of these need the face-shape
cues, which the app reads while **Show expression evidence** is on.

#### Moving

- **What it is**: how fast the face moves in the picture, as the head turns, nods or shifts.
- **Its values**: face widths a second. Under 0.05 is still; 0.1 to 0.5 is ordinary movement; more is a lively or
  restless head.
- **Why it matters**: how still or animated someone is says something about their energy and attention, and a face
  that moves a lot is harder to read.
- **What the app does with it**: shows it and learns each person's usual. The pulse is read only while it is under
  0.15.
- **Where you see it**: the face evidence ("moving"); the Signals chart, Face, "moving" (in Their normal, "head
  moving"); alerts, "Face movement". Key: `motion`.

#### Coming closer

- **What it is**: whether the face is coming toward the camera or moving away, from how fast it grows or shrinks in
  the picture.
- **Its values**: above 0 is coming closer, below 0 moving away, and about 0 staying put. Shown as "in" or "out"
  beside Moving when it is clear.
- **Why it matters**: it tells someone walking up to the camera from someone leaving.
- **What the app does with it**: shows it and puts it in an alert's details.
- **Where you see it**: the face evidence ("moving .40 in"); alerts, "Coming closer". Key: `approach`.

#### Pulse

- **What it is**: an estimate of the heart rate, from the camera alone, switched on with **Webcam pulse
  (experimental)** (off unless you switch it on).
  - With every heartbeat, the skin's colour changes very slightly as blood flows in.
  - The app follows the colour of the forehead and the cheeks over the last 10 seconds and finds the rhythm that
    repeats.
- **Its values**: beats a minute, 42 to 180. At rest most adults are about 60 to 100. "?" or "weak" means the rhythm
  stood out only faintly, and the number in dB beside it says how clearly it stood out. A reading at the very bottom
  of the range (about 42 to 45) more often comes from light or movement than from a heartbeat.
- **Why it matters**: the heart rate rises with movement, effort, excitement or stress, and falls with rest. Compared
  with the person's own usual, a rise is a cue that something changed, not a reason.
- **What the app does with it**: shows it and learns each person's usual. It is read only while the face is still, and
  it needs steady light. It is not a medical reading.
- **Where you see it**: the face evidence ("pulse 72"); the Signals chart, Face, "pulse"; alerts, "Pulse", under "Body
  rhythms (from the camera)". Key: `pulse`.

#### Blinks

- **What it is**: how often the person blinks, counted over the last minute.
- **Its values**: blinks a minute. Most people blink about 10 to 20 times a minute; under 5 is few, and over 30 is
  many. It is a rough count: a blink is short, and some fall between two checks.
- **Why it matters**: people blink less while they concentrate, read or watch a screen, and more when tired, when the
  eyes are dry or strained, or when talking.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the face evidence ("blinks"); the Signals chart, Face, "blinks"; alerts, "Blink rate". Key:
  `blinkRate`.

#### Gaze away

- **What it is**: how much of the time the eyes looked away from straight ahead (left, right, up or down), over the
  last 5 seconds.
- **Its values**: 0% (always looking ahead) to 100% (always looking away). Above 50% is highlighted. "Ahead" is
  toward the camera: with the camera beside a screen or a television, a person watching it looks away most of the
  time, and a high share is usual for that place.
- **Why it matters**: it says where the attention is. People look away while thinking, when something else in the
  room draws them, or when they avoid a look.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the face evidence ("gaze away"); the Signals chart, Face, "gaze away"; alerts, "Looking away".
  Key: `gazeAway`.

#### Smile

- **What it is**: how much the corners of the mouth are pulled up.
- **Its values**: 0 (no smile) to 1 (a broad smile). Most of the time a face reads under 0.1; 0.3 or more is a
  clear smile.
- **Why it matters**: it is the strongest single cue of a happy look, and how often and how broadly a person smiles
  is part of their usual.
- **What the app does with it**: shows it, learns each person's usual, and uses it as a cue for happy.
- **Where you see it**: the Signals chart, Face, "smile"; alerts, "Smile". Key: `smile`.

#### Smile asymmetry

- **What it is**: how lopsided the smiles were over the last 30 seconds: one corner of the mouth pulled up more than
  the other.
- **Its values**: 0 (both sides alike) to 1 (one side only). Under 0.1 is even; 0.2 or more is clearly lopsided.
  Missing when there was no smile.
- **Why it matters**: smiles that come on by themselves tend to be even; a lopsided smile is more often a polite,
  wry or put-on one. It is a hint, not a test.
- **What the app does with it**: shows it and puts it in an alert's details.
- **Where you see it**: the face evidence ("smile asym"); alerts, "Smile asymmetry". Key: `smileAsym`.

#### Smile rise

- **What it is**: how quickly the latest smile came on.
- **Its values**: the smile's fastest rise a second over the last 2 seconds. Shown only when it is quick (above 0.5).
- **Why it matters**: smiles that come on by themselves tend to rise smoothly; one switched on at once is more often
  a put-on one. It is a hint, not a test.
- **What the app does with it**: shows it.
- **Where you see it**: the face evidence ("smile rise").

#### Jaw

- **What it is**: how open the mouth is.
- **Its values**: 0 (closed) to 1 (wide open). In conversation it is usually small, under 0.1; a yawn or a gasp is
  much more.
- **Why it matters**: an open mouth goes with talking, yawning, laughing and surprise.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the Signals chart, Face, "jaw"; alerts, "Jaw open". Key: `jaw`.

Under the face, the "jaw" line says something else: whether the jaw is **moving** (opening and closing, as in
speech) or **still**. The number there is how much the jaw's opening went up and down over the last 1.5 seconds;
0.035 or more counts as moving.

#### Talking

- **What it is**: whether this person is talking right now.
  - It is yes when the jaw moves as in speech and, where the microphone is listened to, the microphone hears a
    voice.
  - Without the face-shape cues, the microphone alone decides, and only when one face is in view.
- **Its values**: yes or no. In alerts, the share of the last 30 seconds spent talking.
- **Why it matters**: it says who is speaking, and talking moves the mouth in ways the expression model can misread
  (as surprised or happy).
- **What the app does with it**: gives the voice readings to the face that is talking, so that one person's voice
  isn't counted as another's. With **Skip expression while talking** on, it leaves out the checks while the person
  talks.
- **Where you see it**: the face evidence ("talking"); alerts, "Talking". Key: `talking`.

#### Cheek raise

- **What it is**: how much the cheeks are pushed up, narrowing the eyes from below.
- **Its values**: 0 to 1. The face model seldom gives this much: it often stays at 0, even in a smile.
- **Why it matters**: a smile that reaches the eyes raises the cheeks; a smile of the mouth alone doesn't.
- **What the app does with it**: shows it, and uses it as a cue for happy.
- **Where you see it**: the Signals chart, Face, "cheek raise"; alerts, "Cheek raise". Key: `cheek`.

#### Frown

- **What it is**: how much the corners of the mouth are pulled down.
- **Its values**: 0 to 1. Usually under 0.1; 0.3 or more is clear.
- **Why it matters**: it is a cue of a sad or displeased look.
- **What the app does with it**: shows it, and uses it as a cue for sad.
- **Where you see it**: the Signals chart, Face, "frown"; alerts, "Frown". Key: `frown`.

#### Brow down

- **What it is**: how much the brows are drawn down and together.
- **Its values**: 0 to 1. Usually under 0.1; 0.3 or more is clear.
- **Why it matters**: it goes with a scowl, but just as often with concentration or bright light.
- **What the app does with it**: shows it, and uses it as a cue for angry.
- **Where you see it**: the Signals chart, Face, "brow down"; alerts, "Brows down". Key: `browDown`.

#### Inner brow up

- **What it is**: how much the inner ends of the brows are raised.
- **Its values**: 0 to 1. Usually under 0.1; 0.3 or more is clear.
- **Why it matters**: it goes with worry, sadness and pleading looks.
- **What the app does with it**: shows it, and uses it as a cue for sad and fearful.
- **Where you see it**: the Signals chart, Face, "inner brow up"; alerts, "Inner brows up". Key: `browInnerUp`.

#### Eyes wide

- **What it is**: how wide open the eyes are, beyond their usual opening.
- **Its values**: 0 to 1. Usually near 0; 0.2 or more is clear.
- **Why it matters**: it goes with surprise, fear and sudden attention.
- **What the app does with it**: shows it, and uses it as a cue for surprised and fearful.
- **Where you see it**: the Signals chart, Face, "eyes wide"; alerts, "Eyes wide". Key: `eyeWide`.

#### The other face-shape cues

Six more cues are read and shown only among the **cues for this look**, beside the label. Each is a score from 0 to 1:

- **press**: the lips pressed together (a cue for angry);
- **stretch**: the mouth stretched sideways (fearful);
- **outer brow up**: the outer ends of the brows raised (surprised);
- **eye squint**: the eyes narrowed (angry);
- **sneer**: the nose wrinkled (disgust);
- **upper lip**: the upper lip raised (disgust).

### Voice

These readings are taken only while **Listen to voice (microphone)** is on. The microphone's sound is measured as it
comes, over the last 3 seconds, and never stored. The readings are given to the face that is talking, or to the only
face in view. A voice changes with a person's energy, effort and the room as much as with anything else: as with the
face, a change from their own usual says the most.

#### Speaking

- **What it is**: how much of the last 3 seconds had a voice in it.
- **Its values**: 0% (silence) to 100% (talking all the time). Above 10% the other voice readings are shown.
- **Why it matters**: it says how much the person talks, and whether there is enough voice for the readings below.
- **What the app does with it**: shows it. The voice model below is asked only when there is enough voice.
- **Where you see it**: the voice evidence ("speaking"); the Signals chart, Voice, "speaking"; alerts, "Voiced
  share". Key: `speaking`.

#### Loudness

- **What it is**: how loud the voice is compared with the room's own background noise.
- **Its values**: dB above the background. A voice counts only from about 10 dB above it, and most speech reads 10
  to 25; more is loud. This is not the sound's absolute loudness: the same voice reads lower in a noisy room, and
  lower further from the microphone.
- **Why it matters**: people speak up when excited, angry, emphatic or far away, and softly when tired, unsure or
  being quiet on purpose.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the voice evidence ("+12 dB"); the Signals chart, Voice, "loudness"; alerts, "Loudness". Key:
  `loudness`.

#### Pitch

- **What it is**: how high or low the voice is.
- **Its values**: Hz, the typical pitch over the last 3 seconds. Most men's speaking voices lie around 85 to 155 Hz
  and most women's around 165 to 255 Hz. Higher is a higher voice.
- **Why it matters**: a person's pitch tends to rise with excitement, effort or stress, and drop when they are calm or
  tired. Each person has their own, so only a change from it says anything.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the voice evidence ("140 Hz"); the Signals chart, Voice, "pitch"; alerts, "Voice pitch". Key:
  `pitch`.

#### Pitch range

- **What it is**: how much the voice's pitch goes up and down while speaking: lively or flat.
- **Its values**: semitones. Under about 2 is flat and even; 3 to 6 is ordinary, lively speech; more is very
  animated.
- **Why it matters**: a flat voice can go with tiredness, boredom or reading aloud; a wide range with engagement,
  excitement or a story being told.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the voice evidence ("pitch range ±1.2 st"); the Signals chart, Voice, "pitch range"; alerts,
  "Pitch variation". Key: `pitchSd`.

#### Speech rate

- **What it is**: how fast the person speaks, counted as the loud beats of the voice, roughly one for each syllable.
- **Its values**: about syllables a second, while speaking. Ordinary speech is about 3 to 6; slower is careful or
  tired, faster is hurried or excited.
- **Why it matters**: people speed up when excited, nervous or in a hurry, and slow down when tired, thoughtful or
  explaining.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the voice evidence ("rate"); the Signals chart, Voice, "rate" (in Their normal, "speech
  rate"); alerts, "Speaking rate". Key: `speechRate`.

#### Voice arousal

- **What it is**: how calm or how energetic and worked up the voice sounds, as the app's voice model, Melos 1, hears
  it. The model learned from many people's judgements of how voices sounded.
- **Its values**: about 0 (calm, flat, tired) to 1 (energetic, excited, agitated). Most speech reads about 0.45 to
  0.75.
- **Why it matters**: of the three, a voice shows arousal most clearly. It rises with excitement, anger, fear and
  enthusiasm alike, so it says how much energy there is, not which kind.
- **What the app does with it**: shows it and learns each person's usual. Read only while there is enough speech;
  without the voice model there is none.
- **Where you see it**: the voice evidence ("voice A.62"); the Signals chart, Voice, "arousal" (in Their normal,
  "voice arousal"); alerts, "Voice arousal". Key: `voiceArousal`.

#### Voice valence

- **What it is**: how pleasant or unpleasant the voice sounds, from the same model.
- **Its values**: about 0 (unpleasant) to 1 (pleasant). The middle of the scale is not 0.5: most speech reads about
  0.2 to 0.4, so 0.3 is an ordinary voice, and only a value well above or below the person's usual says anything.
- **Why it matters**: a warm, friendly voice reads higher than a harsh or sullen one. A voice carries this less
  clearly than arousal, since much of it is in the words, which the app doesn't listen to.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the voice evidence ("V.41"); the Signals chart, Voice, "valence" (in Their normal, "voice
  valence"); alerts, "Voice valence". Key: `voiceValence`.

#### Voice dominance

- **What it is**: how forceful and in control the voice sounds, from the same model.
- **Its values**: about 0 (timid, hesitant) to 1 (forceful, commanding).
- **Why it matters**: it tells a confident or insistent voice from an unsure or deferring one.
- **What the app does with it**: puts it in an alert's details.
- **Where you see it**: alerts, "Voice dominance". Key: `voiceDominance`.

### Body and hands

These readings are taken only while **Body posture** is on, from the body points of the person whose face it is. Most
of them need both shoulders in view. Posture and movement often say as much as the face about someone's energy, and
less about whether they are pleased.

#### Head height

- **What it is**: how high the head is above the shoulders.
- **Its values**: shoulder widths, from the shoulders' middle up to the nose. About 0.4 to 0.8 is sitting up; lower
  means the head is down or the person is slumped. It depends on the camera's height too, so compare it with the
  person's usual.
- **Why it matters**: a lowered head or a slump can go with tiredness, low energy or reading something below; an
  upright head with alertness.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the body evidence ("head"); the Signals chart, Body and hands, "head height"; alerts, "Head
  height". Key: `head` (`headHeight` in an alert's last 30 seconds).

#### Neck

- **What it is**: how far the ears are above the shoulders.
- **Its values**: shoulder widths. About 0.5 to 0.8 is relaxed; smaller means the shoulders are raised toward the
  ears.
- **Why it matters**: raised shoulders go with a shrug, with tension, with cold, or with hunching over a desk.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the body evidence ("neck"); the Signals chart, Body and hands, "neck"; alerts, "Neck angle".
  Key: `neck`.

#### Shoulder tilt

- **What it is**: how far the line of the shoulders leans from level, to either side.
- **Its values**: degrees. Under about 10 is level enough; 20 or more is leaning clearly to one side.
- **Why it matters**: it shows leaning on an elbow or an armrest, slouching to a side, or reaching for something.
- **What the app does with it**: shows it and learns each person's usual.
- **Where you see it**: the body evidence ("tilt 4°"); the Signals chart, Body and hands, "shoulder tilt"; alerts,
  "Shoulder tilt". Key: `tilt` (`shoulderTilt` in an alert's last 30 seconds).

#### Leaning

- **What it is**: whether the person leans toward the camera or away from it, from how much wider or narrower their
  shoulders look than over the last 30 seconds.
- **Its values**: leaning in, leaning back, or upright.
- **Why it matters**: people lean in when interested or looking closely, and back when relaxed or withdrawing.
- **What the app does with it**: shows it.
- **Where you see it**: the body evidence ("lean").

#### Body movement

- **What it is**: how much the shoulders and wrists move.
- **Its values**: shoulder widths a second. 0 is still; the more the shoulders and hands move, the higher. Hands
  moving fast in a gesture read far higher than a body shifting in a chair, so compare it with the person's usual.
- **Why it matters**: how much a body moves follows energy (arousal) closely: restless, animated or excited people
  move more, and calm, tired or focused ones less.
- **What the app does with it**: shows it and learns each person's usual. Breathing is read only while it is under
  0.1.
- **Where you see it**: the body evidence ("movement"); the Signals chart, Body and hands, "movement" (in Their normal,
  "body movement"); alerts, "Body movement". Key: `bodyMotion`.

#### Breathing

- **What it is**: an estimate of the breathing rate, from the shoulders rising and falling over the last 30 seconds.
- **Its values**: breaths a minute, 6 to 30. At rest most adults breathe about 12 to 20 times a minute. "?" or "weak"
  means the rhythm stood out only faintly. A reading at the very bottom of the range (6 to 7) more often comes from
  slow shifting than from breathing.
- **Why it matters**: breathing speeds up with effort, talking, excitement or stress, and slows when relaxed or
  asleep.
- **What the app does with it**: shows it and learns each person's usual. It is read only while the body is still. It
  is not a medical reading.
- **Where you see it**: the body evidence ("breathing 14/min"); the Signals chart, Body and hands, "breathing"; alerts,
  "Breathing", under "Body rhythms (from the camera)". Key: `breathing`.

#### Hand on face

- **What it is**: whether a wrist or a fingertip of this person is over their face.
- **Its values**: yes or no. In alerts, the share of the last 30 seconds.
- **Why it matters**: people touch their face when thinking, tired, unsure or resting their head. A hand also hides
  part of the face and can spoil the expression reading.
- **What the app does with it**: shows it. With **Skip expression when a hand is on the face** on, the app leaves
  those checks out.
- **Where you see it**: the body evidence ("hand on face"); the Signals chart, Body and hands, "hand on face"; alerts,
  "Hand on face". Key: `hand` (`handOnFace` in alerts).

#### Arms crossed

- **What it is**: whether the arms are folded across the body: both wrists below the shoulders, each on the other
  side.
- **Its values**: yes or no. In alerts, the share of the last 30 seconds.
- **Why it matters**: crossed arms are often read as closed or guarded, but just as often mean someone is cold,
  comfortable or waiting.
- **What the app does with it**: shows it.
- **Where you see it**: the body evidence ("arms crossed"); the Signals chart, Body and hands, "arms crossed"; alerts,
  "Arms crossed". Key: `armsCrossed`.

#### Finger points

- **What it is**: the 21 points of each hand (the joints and the tips of the fingers), drawn on the picture while
  **Hand skeleton (fingers)** is on.
- **Its values**: none. They are drawn only, and nothing of them is kept.

---

## Their normal

With **Calibrate to each person** on, the app learns each named person's usual level of 17 readings:

- the face: moving, pulse, blinks, gaze away, smile and jaw;
- the voice: loudness, pitch, pitch range, speech rate, voice arousal and voice valence;
- the body: head height, neck, shoulder tilt, body movement and breathing.

Each of these readings is then shown against that usual. This matters because people differ: one person's resting
pulse, pitch or blink rate is another's excited one. So a reading says the most when it is compared with the same
person on other days. The usual is kept on the device and shared with your other devices. **Forget what's learned**
clears it.

#### Their usual

- **What it is**: the person's own average and spread of one reading.
  - It is learned from one sample a second while they are in view.
  - For the first 10 minutes or so it is a plain average; after that the newest 10 minutes or so count the most, so
    it follows slow changes.
- **Its values**: the reading's own units. In the Signals chart it is the green band (their average, give or take
  2 spreads); a reading outside it is drawn orange.
- **Why it matters**: it is what "high" and "low" mean for this person.
- **What the app does with it**: measures every new reading against it.
- **Where you see it**: the green bands of the Signals chart; alerts, "Their usual" beside each reading.

#### Learning

- **What it is**: how far the app has come in learning a person's usual. Each reading's usual is trusted after about
  2 minutes of that reading.
- **Its values**: 0% to 100% ("learning 40%").
- **Why it matters**: until a usual is learned, nothing can be called unusual for that person.
- **What the app does with it**: doesn't compare a reading with a usual it hasn't learned yet.
- **Where you see it**: the normal evidence ("learning 40%"); alerts, "Their normal learned". Key: `normalLearned`.

#### Alone or with others

- **What it is**: whether the person is alone in view (one face in the picture) or with others.
- **Its values**: alone or company.
- **Why it matters**: people often look, sound and sit differently in company: they smile and talk more, and move
  differently. So the app learns a usual for each.
- **What the app does with it**: uses the usual that fits, once it is learned, else their usual over all.
- **Where you see it**: the normal evidence ("usual (alone)"); alerts, "Alone or with others". Key: `setting`.

#### Spreads away

- **What it is**: how far a reading is from the person's usual, counted in their spreads.
- **Its values**: 0 is their average; +1 a little above; −1 a little below; ±2 is unusual for them; ±3 or more is
  rare for them.
- **Why it matters**: it puts every reading on one scale, so you can see which ones stand out for this person, not
  for people in general.
- **What the app does with it**: shades the Their normal chart (darker is further from usual) and keeps, for every
  10 seconds, the furthest a reading went.
- **Where you see it**: the Signals chart, Their normal; alerts, the "Spreads away" column. Key: `z`.

#### Unusual signals

- **What it is**: how many of the person's readings are unusual for them right now (2 spreads or more away).
- **Its values**: a count. "unusual · 3 signals" when at least 3 are; "usual" when none.
- **Why it matters**: one reading out of its range happens all the time; several at once say that something about the
  person or the situation is different.
- **What the app does with it**: marks "Ann: unusual signals" in the chart's events when 3 or more stay unusual for
  5 seconds. The marks are not alerts.
- **Where you see it**: the Signals chart, Their normal. Key: `odd`.

#### Most unusual

- **What it is**: the reading that is furthest from the person's usual now, and the next ones.
- **Its values**: the reading and how many spreads away it is, such as "head −3.1 SD" (3.1 spreads below their usual
  head height).
- **Why it matters**: it says which reading to look at first.
- **What the app does with it**: shows it.
- **Where you see it**: the normal evidence ("most unusual", "next").

#### The unusual alert

- **What it is**: an alert, **Their signals are unusual for them** (off unless you switch it on), for the people on
  your alert list.
- **Its values**: it is raised when any of a person's readings has stayed far from their usual for a while. The
  settings are how far (3 spreads unless you change it), for how long (30 seconds) and how often at most (once in
  30 minutes for each person).
- **Why it matters**: it lets you know when someone's readings stay well away from their own usual, without watching
  the chart.
- **What the app does with it**: raises the alert with the three readings furthest from usual.
- **Where you see it**: Alerts.

---

## Where the readings are kept

- **The Signals chart** keeps the last 15 minutes in the page's memory only. Close the page and they are gone.
- **An alert** keeps what was measured when it was raised.
- **The signal history** (with **Keep a history of every signal** on, and a shared database) keeps, for every
  10 seconds, each reading's average, lowest, highest and count, for the device and for each named person in view.
  It keeps them for the days you set.
- **The expression log** (with a shared database) keeps the expression label and its numbers every few seconds.

Only numbers and short words are kept, never a picture or a sound. A person's readings go with their export and are
deleted when they are forgotten. Settings deletes a device's history.
