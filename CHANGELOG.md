# Changelog

## 2.6.0 (2026-11-24)

- **New: where each device is.** A device can show its location: the nearest town with its state and country,
  the coordinates, how sure the reading is and how old. It is shown in a new **Location** card on the device's
  Face tracking tab and, on your other devices, over that device's picture in the Cameras tab. It is off until
  you switch it on, on each device by itself (**Show and keep this device's location**: Settings, Face
  tracking: location; on a phone, in its Controls). While it is off the app asks no location service, shows no
  location and tells your other devices none.
- **Where the location comes from.** The device's own location service: Windows, a Mac, an iPhone and an
  Android phone each ask you once, when you switch it on. Or a place you type, which asks nobody: a town
  (`Springfield, Illinois`, `Paris, France`) or latitude and longitude. A computer without GPS is placed by
  its system from the networks around it, often only to within a few kilometres, so a computer that stays
  where it is does better with a typed place. Linux and a Raspberry Pi have no location service, so there a
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
