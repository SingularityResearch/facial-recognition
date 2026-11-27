# Changelog

## 2.9.1 (2026-11-27)

- **Several cameras fill the window.** On a wide window the Cameras tab shares its whole height between the rows,
  and each camera's picture takes all the room its readout leaves, so nine cameras show as three rows of three with
  no space left over. A row with an opened Signals chart is as tall as the chart needs.
- **A move alert names devices as you do.** "Moved from one camera to another" names the camera the person arrived
  at by the device's name in Settings, as every device shows it, never by the computer's own name.
- A long group heading in a camera's readout ends in an ellipsis instead of widening the readout.
- The README links to [a 13-minute video tour](https://youtu.be/xnv65x_Kbl4) of the app.

## 2.9.0 (2026-11-26)

- **What each reading means**: [docs/READINGS.md](docs/READINGS.md) explains every reading the app takes, from the
  face, the voice, the body, the room's sound and the device itself: what it is, what its values mean, why it
  matters, what the app does with it and where you see it. Words like valence and arousal are explained. The app
  carries the same page: **What each reading means** on the Face tracking and Cameras tabs, and a reading's name in
  the Signals chart or a camera's readout opens it at that reading.
- **The Cameras tab reads more easily.** Each person's readout beside a camera starts with a summary (the
  expression, what stands out now, such as talking, looking away or unusual for them, and the key numbers), and
  their readings follow group by group, each opened when you want it; every reading says what it is when you point
  at it. **Focus** shows one camera with every reading laid out in full. The cameras shown are chosen in a list
  that opens from the header, with **All cameras** or one by one.
- **Setup is at the top of the Settings tab**, no longer in the tab bar, and **Check for updates** is above it with
  the app's version (the tab bar shows the version only).
- The expression's lead is shown as a percentage ("lead 21%").

## 2.8.0 (2026-11-26)

- **New: the Analyze tab.** Ask in your own words, typed or spoken, about anyone or any signal, now or over time.
  The language model of Describe photo writes its own read-only SQL over your shared database's records, draws
  trend charts with all the history the question covers, and shows the pictures it finds a page at a time, by
  relevance or by time. Speech is made into words on the computer itself (a speech model downloaded when you ask
  for it); the conversation is never stored.
- **New: Setup.** A **Setup** button in the window's tab bar shows what is set up on this device and what is left:
  your photos, your devices and their shared database, the model servers for Describe photo and Analyze, the
  optional keys, your network. What is set up is marked done and left as it is; each step it can do is a button
  that says what it does. It opens by itself on a new install, and once after an update to say what the update
  changed on that device.
- **The setup guide is in the app.** Setup's **Guide** shows it, and a button beside each step opens it at that
  step. The same guide is [docs/SETUP.md](docs/SETUP.md).
- **New: a shared database on this computer, at one button.** Setup, Your devices, **Set up a new one on this
  computer** (Windows, a Mac and Linux): the app downloads PostgreSQL with pgvector (from conda-forge, each package
  checked by its size and SHA-256), makes the database with certificates of your own, starts it and connects. No
  administrator rights, no service, no account. It keeps running when the app is closed and starts again when you
  sign in. Installed again, the app finds it; with a lost private folder, **Take it on** connects to it again.
- **Connect prepares a new server by itself.** Given another server's administrator login once, the app makes its
  own login, the database, pgvector and the tables, registers your certificate authority there, and keeps only its
  own login. Settings makes the server's certificates under an authority of yours, with the lines its settings need.
- **The licence is shown first.** Windows Setup shows it on a page of its own; on a Mac, Linux and a Raspberry Pi
  the app shows it at its first start. Accept to go on; declining ends Setup or the app.
- **Settings are grouped by what each one is for,** with a row of buttons at the top for the groups. All the AI
  settings are together; **Model servers on this computer** lists every kind found here (LM Studio, Ollama,
  llama.cpp, vLLM), **Model servers on your other devices** every kind on each device.
- **Keeping this computer's model servers open to your other devices is a switch now** (off for a new install; on
  where a shared database was set up before), and it covers Ollama, llama.cpp and vLLM as well as LM Studio.
- **Fixed: face recognition didn't start on a Windows computer without Microsoft's Visual C++ runtime.** The app
  carried the runtime but Windows didn't find it; it does now.
- The privacy policy was revised (25 November 2026): the database the app can set up on your computer, and what
  it downloads for it.

## 2.7.0 (2026-11-25)

- **New: a history of every signal, for analysis.** A device can keep what it reads in your shared database:
  one row every 10 seconds for the device itself (how fast it checks, faces in view, processor, memory,
  temperature, battery, what its sound interfaces hear) and one for each named person in view (recognition,
  expression, face, voice, body and hands, the pulse estimate and breathing rate, and how far each was from
  that person's usual). A row holds each signal's average, lowest and highest in those 10 seconds, as numbers
  and short words: never a picture or a sound. It is off until you switch it on, on each device by itself
  (**Keep a history of every signal**: the History card of the Face tracking tab, or Settings, Face tracking:
  history; on a phone, in its Controls). Without a shared database nothing is kept.
- **You set how long it is kept.** **Keep it for (days)**, 90 unless you change it, 1 to 3650. Each device
  deletes its own older rows once a day, and its location history now follows the same days.
- **It is written beside the camera's work, not in it, and as fast as that allows.** The page hands each
  row over as soon as it is finished, and one part of the app that does nothing else, with a database
  connection of its own, writes it: at once while nobody is in view, and while somebody is in view as often
  as keeps its database work under a thousandth of the time, so a row is usually in the database a second or
  two after its 10 seconds end. A database that is slow or far away is written to less often, by itself.
  While the database can't be reached the rows wait on the device. Nothing else in the app reads the history
  or waits for it.
- **Your photos and videos in the history.** Settings, Labeling: history, **Work out the history from the
  library**: one row for each named person in each photo and in each video frame a face was found in, at the
  picture's own time, with how the face looks, how good its picture is and how many people the picture has.
  A still says less than a camera, and the rows say so. It runs in the background like an import, follows
  the labels when it is run again, and runs by itself after a folder is added where the device's history is
  on.
- **The expression readings and the music log are kept for the same days.** Until now they were kept without
  an end. Each device deletes its own rows that are older than its days (90 unless you change them), once a
  day, whether or not its history is on: on a device that has kept them longer, the older rows go at the
  first tidying.
- **Describe photo with Ollama, llama.cpp or vLLM.** Settings, Labeling: Describe photo has a **Kind of
  server**: LM Studio, Ollama, llama.cpp's server or vLLM. Each is asked for its models in its own way (which
  of them read pictures, which is loaded) and shows only the fields it needs: its usual address, the model,
  and an API key where it takes one.
- **Settings suggests the model and settings for this computer, and loads it.** Under Describe photo,
  **Suggested for this computer** names the model that suits the computer's graphics memory on the kind of
  server chosen, with its settings (and for llama.cpp and vLLM the command to start them with); choosing the
  kind or the model, or **Load it**, loads it (LM Studio on this computer, Ollama anywhere). It never downloads
  a model. The measurements behind it, for five computers and four servers, are in
  [docs/MODELS_AND_HARDWARE.md](docs/MODELS_AND_HARDWARE.md), with charts, together with how well and how fast
  the app's own models, Opsis 1 and Melos 1, work on each computer.
- **Share Describe all with every kind of server.** Your other computers' Ollama, llama.cpp and vLLM servers now
  take photos too, as LM Studio's did: of the same kind and with the same model.
- Forgetting a person deletes their rows of the history, and their export holds them. **Copy settings from**
  never copies the history's switch or its days to another device.
- The privacy policy was revised (25 November 2026): the signal history, the days each device keeps its rows,
  and the photos Describe all shares with your other computers' servers.

## 2.6.0 (2026-11-24)

- **New: where each device is.** A device can show its location: the nearest town with its state and country,
  the coordinates, how sure the reading is and how old. It is shown in a new **Location** card on the device's
  Face tracking tab and, on your other devices, over that device's picture in the Cameras tab. It is off until
  you switch it on, on each device by itself (**Show and keep this device's location**: Settings, Face
  tracking: location; on a phone, in its Controls). While it is off the app asks no location service, shows no
  location and tells your other devices none.
- **Where the location comes from.** The device's own location service: Windows, a Mac, an iPhone and an
  Android phone each ask you once, when you switch it on. Or a place you type, which asks nobody: a town
  (`Springfield, Illinois`, `Paris, France`) or latitude and longitude. A computer has no GPS: its system
  places it by the Wi-Fi networks around it, so it needs its Wi-Fi switched on, and it says so beside its
  place. Without Wi-Fi a Windows computer is placed only roughly (to within kilometres) and a Mac not at all,
  and each says that too: switch Wi-Fi on, or type the place. Linux and a Raspberry Pi have no location service, so there a
  place is typed. The town's name comes from a list of 70,000 towns inside the app (from GeoNames): nothing is
  looked up on the internet.
- **A location history, in your shared database.** A row is written when you switch the location on, and then
  each time the device has moved more than about 100 m, at most one a minute; a device that stays where it is
  adds nothing. Rows are deleted after 90 days. Settings, Shared database shows a device's history (**Location
  history of**) and deletes it (**Delete a device's location history**). Without a shared database nothing is
  written. A phone sends its rows itself, and keeps those it could not send until it is back on your network.
- The phones' apps ask for the location permission, only when you switch the location on. The Mac's app asks
  macOS for it the same way. The privacy policy says what the system's location service does.
- **Copy settings from** never copies a device's location switch or its typed place to another device.
- **Fixed: a computer could switch to another camera for good.** When the camera you picked wasn't there at
  a start, the page opened the default camera and remembered that one in its place. On a computer with a
  second camera that shows nothing (a phone offered as a webcam while the phone is away gives a black
  picture), every later start showed a blank picture until the camera was picked by hand again. The camera you
  picked now stays the device's camera: the default one is used only meanwhile, and the picked one is gone
  back to as soon as it is there again.

## 2.5.2 (2026-11-24)

- **The licence is now Version 1.1** (24 November 2026): one section was removed and the others renumbered.
  What you may do with the app and its source is unchanged.
- The privacy policy was revised (24 November 2026), and the documents and on-screen hints were brought in
  line with what the app has: they no longer describe a download for NVIDIA cards or tools that are not part
  of the app.
- **Faster with a face in view: the face, body and finger readings no longer hold the page.** With the
  options that read a face's cues, mesh, posture, body and fingers switched on, those readings ran in the
  tracker page itself and held it for a tenth of a second and more with every check: the checks fell from 40
  or more a second with nobody in view to 5 to 10 with a face, on the fastest computers too. They now run
  beside the page: on a computer each of the three models has its own worker and the three run at the same
  time, and the body and the hands are looked for while the frame is still being read for faces. Measured with
  a face in view, checks a second: with all five readings on, 4.4 before and 10.1 now on a Windows computer,
  9.2 and 19.8 on a Mac; with cues and posture alone, 8.3 and 14.4 on the Windows computer. The readings
  themselves are the same to the last digit (compared on the same frames, in both browser engines). A phone
  and a Raspberry Pi get one worker for all three: on an iPhone 12 Pro Max that lifted the camera's stream
  with a face in view (about 5.7 to 7.4 frames a second) and left the checks where they were, since the
  phone's processor is full either way.
- **Fixed: a camera's stream fell to about 10 frames a second while it had a face in view.** Each frame for
  your other devices had to wait for the busy page four times on its way out, and the Cameras tab showed that
  camera at 9 to 12 frames a second where it is meant to have 20. The frames are now made and sent beside the
  page, which only hands each picture over. With the readings beside the page as well, a camera keeps its 20
  frames a second with a face in view: on Windows, Linux and a Mac (measured there: 15.3 a second before);
  an iPhone sends its frames as before, because on the phone it was measured on the new way gave about a
  tenth more frames and cost checks.
- Fixed: a model asked for twice while it was still being loaded answered the second time as if nobody were in
  the picture.

## 2.5.1 (2026-11-23)

- **Fixed: the log filled up with one line said over and over.** A sound library on Linux wrote the same
  error line about fifty times a minute while the sound chart was on, and a device that could not be reached
  was reported every twenty seconds. A line said again in the same words is now written once a minute, with
  how often it was said meanwhile.
- **A removed device stays removed under another id.** Removing a device on this device now remembers the
  certificate that device had, and a machine showing that certificate is refused whatever id it names. Before,
  whoever had the shared database's login could have written a row with a new id for a removed machine.
  Adding the device back forgets the certificate.
- **The alert for a new device no longer trusts the database's dates.** Whether a device is new is now what
  this device itself has seen, not the date in the device's row (which whoever writes the row sets). A device
  you know that shows another certificate than before now raises an alert too, once: it was set up again, or
  another machine uses its id.
- **Who is in view goes to the other devices only while the camera is streamed.** The readings a device
  publishes to your other devices said who was in view and how they looked also with **Stream this camera to
  my other devices** switched off. They now go out only while it is on, and only to the devices listening at
  that moment; they are no longer kept for a device that asks later.
- **Fixed: the Cameras tab's pictures could stall with several cameras.** Each camera's chart held a
  connection of its own to the app, and a browser gives a page only a few: with more cameras than that, the
  pictures waited. The page now asks once for all cameras.
- **Fixed: a chart could jump or lose readings after a connection was made again.** Readings that arrived
  late in one batch were taken for a clock that had been set, and the chart forgot what it had. Each reading
  is now placed at its own time; a clock that really was set is noticed after half a minute.
- **A device asks your other devices only on a local network.** The list of members is filled from the shared
  database, which anyone with its login can write to; an address out on the internet in it is no longer asked,
  unless **Fully distributed system** is on. A device that is no longer in the database is no longer a member,
  and a device whose database login is refused stops answering as one.
- **What other devices and the database send is read more carefully.** A people file from another device is
  read by a reader that takes only what such a file has and checks every size first; a file that can't be used
  is asked of the next device. A device's row with something missing is left out and no longer ends the sync
  step. A question to the app whose wait is no number no longer ends in an error, and one that names a
  device by something that is no device id is refused.
- **The service between devices holds up under load.** A connection that sends nothing is ended after ten
  seconds, one address has a share of the connections and not all of them, one people file is given at a time
  (read once and kept, not read again for every question), a stream ends when the service is stopped, when its
  device is removed or is no longer a member, and what a device publishes can no longer grow past its limit by
  many small changes.
- **Fixed: a subscriber to part of what a device publishes was answered four times a second with nothing**,
  whenever readings were published elsewhere. It now waits until there is something for it.
- Fixed: the one-copy check did not cover a first start that still has to ask about its downloads.
- Fixed: on Linux, a wireless mouse's or keyboard's battery was shown as the computer's.
- Fixed: the device's certificate could be made twice when the app's two programs started at the same moment.
- Fixed: the Cameras tab asks the shared database where a device is when the place it knew no longer answers.

## 2.5.0 (2026-11-23)

- **A phone names people without the shared database.** A device that keeps no library of its own (a phone)
  now takes the people it can name from one of your computers directly, and asks the shared database only
  when no computer gave them. So a phone goes on naming people while the database can't be reached. Only a
  member of your cluster is given them, each side checks the other's certificate, and the phone holds them in
  memory only, as before.
- **The Cameras tab without the shared database.** Where each of your devices is, and which certificate it
  must show, now comes from the list of members the device keeps itself. When the database can't be reached,
  the Cameras tab lists your devices from that list and goes on showing their cameras. Tried with the
  database stopped: every device went on following every other and showing its cameras, the app started
  without the database began sharing by itself, and each device synced again by itself when the database was
  back.
- **Fixed: two copies of the app could run at once.** Started twice within a few seconds on a slow computer
  (a Raspberry Pi), the second copy did not see the first, which was still loading, and ran beside it on the
  same library; its window then failed and the ordinary browser opened instead, asking for a keyring
  password. The app now takes its place as the one running copy before anything else, and a second copy
  ends having started nothing.
- With the shared database away, what the pages ask of it is answered at once, not after a connection
  attempt each time, so the Cameras list no longer takes many seconds to appear.

## 2.4.0 (2026-11-23)

- **Licence.** From this version on Face Recognizer is a proprietary, commercial product under the
  [Singularity Research Commercial License](LICENSE). The source stays public to be read, and the apps
  Singularity Research publishes may be installed and used on the terms shown where you get them. Anything
  else, building your own copy included, needs a licence from Singularity Research.
- **Name a device.** What you call a device, kept in the shared database, so every device shows it under that
  name: in the Cameras tab, in alerts and in Settings. Set it in Settings → Shared database (**Name a device**:
  any device can name any device), or on the device itself, on its **Join a new device** page.
- A name is one device's alone, as an email address is one account's: a name another device goes by already is
  refused, whatever its capitals.
- A device that joins calling itself what another goes by already (two phones both say "iPhone") is given a
  number by itself, "iPhone 2", and its own page then asks whoever is at it for a name.
- The name is only ever shown. It has nothing to do with how the devices find each other on the network.
- **The devices publish to each other directly (the first part).** Every device now offers the same subscribe
  interface to your other devices: one document of what it is, every sensor it has, what is switched on and
  off, the settings of its Face tracking tab and its live readings, followed by each change as it happens. It
  goes from device to device with no server between them. Only a member is answered, and each side checks the
  other's certificate. The devices now also keep their own list of members, so they go on finding each other
  while the shared database can't be reached. Nothing of it is stored.
- **A camera's chart moves with its device.** A device publishes its readings as it takes them, a few times a
  second, and the Cameras tab is told the moment it does. Before, another device's readings arrived once a
  second with its readout. (The pictures and the list of faces still come the way they did; they move next.)
- **What is new, and what it builds on.** The README says what in this project is its own work, and
  [docs/REFERENCES.md](docs/REFERENCES.md) names the research articles it builds on, each with what the project
  takes from it.
- **Signals: the device itself.** A new section, **Device**, for this device and for every camera of the Cameras
  tab: what the device is (a phone's model, its system, the app's version, its processor and memory, where each
  model runs), and
  how much of its processor and memory is in use, how warm it is, its battery and whether it is on power. Only
  what the device's own system says: degrees on Linux and the Raspberry Pi, the system's heat state on a phone,
  no temperature on a Mac or on Windows. The Camera section says what the camera gives (its picture's size and
  rate). Kept in memory only, like every reading of the chart.
