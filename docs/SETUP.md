# Setting up Face Recognizer, step by step

This guide takes you from a computer that has never had Face Recognizer on it to every feature working, on one
device or on several. Each part says what the app does for you and what you do yourself. Every setting it names is
in the app window's **Settings** tab (Ctrl+3); [SETTINGS.md](SETTINGS.md) describes each one, and
[READINGS.md](READINGS.md) explains every reading the app shows.

What you need depends on what you want:

| You want | You need |
|---|---|
| Faces named on one computer's camera, from your own photos | Part 1 and 2: the app alone |
| Photos described, questions answered about your records (Describe photo, Analyze) | Part 5: the app's built-in model for describing, or a model server for both |
| Several devices that share names, labels and history, watch each other's cameras, phones | Part 3 (once) and part 4 (each further device): a shared database |
| Web search on a face, music as a signal | Part 6: your own keys |

The app has this guide built in: the **Setup** button at the top of the Settings tab shows what is set up on this device
and what is left, and does each step it can for you at a button you press (making the certificates, preparing a
new database server, choosing a model server, downloads). It opens by itself at a new install's first start.

Already set up? An update changes nothing of what works, and the Setup page says once what it changed on your
device. A device that has a shared database keeps it, and
Settings shows the setup steps below only on a device that has none.

---

## 1. Install the app

Download the file for your device from the [latest release](../../../releases/latest):

| Device | File | How |
|---|---|---|
| Windows 10 or 11 (64-bit) | `FaceRecognizer-Setup-x64.exe` | Run it. It installs for your user only, without administrator rights. Windows may warn about a new publisher (SmartScreen): **More info**, then **Run anyway**. Where Microsoft's WebView2 runtime is missing, Setup asks before it gets it. |
| Mac with Apple Silicon, macOS 14 or later | `FaceRecognizer-macos-arm64.dmg` | Open it and drag Face Recognizer into Applications. The app is signed and notarized by Apple. |
| Linux (x86-64, Ubuntu 22.04 or later) | `FaceRecognizer-linux-x64.run` | `sh FaceRecognizer-linux-x64.run` in a terminal; add `--autostart` to start it at sign-in. |
| Raspberry Pi 4 or 5, 64-bit system | `FaceRecognizer-raspberrypi-arm64.run` | `sh FaceRecognizer-raspberrypi-arm64.run`; with `--autostart` it starts at sign-in, which on a Pi is at boot. |
| Android phone (64-bit ARM, Android 8 or later) | `FaceRecognizer-android-arm64.apk` | Open it on the phone and allow installing from that source when Android asks. A phone keeps no photo library: it names faces with a computer's people, through a shared database (part 4). |

An update installs over the app in the same way and never touches your models or your data.

## 2. First start

1. Start Face Recognizer. On a Mac, Linux and a Raspberry Pi it first shows its licence: **Accept** to go on
   (**Decline** ends it; on Windows Setup asked already). Then it asks to download the face models and MediaPipe's
   runtime for the tracker page (about 90 MB). Say **Download**: each file is checked by its size and SHA-256. They go into the app's own folder (Settings, App, shows where).
2. Allow the camera when the system asks (and the microphone, if you want voice readings). On a Mac, System
   Settings, Privacy & Security, Camera, lists the app.
3. Add your photos: the Labeling tab, **Add photos**, and a folder. Name the faces it finds; the more photos of a
   person, the better they are recognized.
4. The Face tracking tab now names the people it knows.

Everything so far stays on this computer. The rest is optional.

---

## 3. A shared database for several devices (once)

Your devices share names, labels, descriptions and history through one PostgreSQL database with the pgvector
extension, on a computer of yours that is on most of the time (a desktop, a NAS, a small server). The app talks to
it only over an encrypted connection that it checks against a certificate authority of yours, which the app makes
for you.

### On this computer: the app sets it up (Windows, Mac and Linux)

The easiest way: the computer the app runs on holds the database, and the app does every step itself.

1. **Setup** (at the top of the Settings tab), **Your devices**, **Several devices**, **Set up a new one on this computer**,
   then **Install and set it up**.
2. The app downloads PostgreSQL with pgvector from conda-forge (about 55 MB on Windows, 30 MB on a Mac, 45 MB on
   Linux, each file checked by its size and SHA-256), makes the database in the app's folder (`database` beside `data`), makes
   certificates of your own for it, starts it, and connects this device. No administrator rights are needed and
   nothing is installed into the system. It takes a minute or two.
3. Windows asks twice whether a program may be reached on your network (Face Recognizer, then PostgreSQL Server):
   each time **Show more**, untick **Public networks**, tick **Private networks**, **Allow** (part 7).
4. Optionally, set a passphrase for keys (part 3.5).

The database keeps running when you close the app, and starts again when you sign in to this computer, so choose a
computer that is on, and signed in to, most of the time. Its data stays when the app is updated or uninstalled
(uninstalling stops it). Your other devices join it with a code (part 4). (Not on a Raspberry Pi: hold the
database on another computer.)

Installed the app again? It finds the database it held and starts it. If the app's private folder was lost on the
way (a new user account, a cleaned profile), Setup shows **Take it on**: the app gives its own login there a new
password, starts it and connects; nothing in the database changes. If the database's folder itself is gone, Setup
says so and offers **Forget it**, after which you can set a new one up or join your other devices' one.

Another computer, a NAS or a server of your own: the steps below.

You need: PostgreSQL 13 or later and pgvector 0.5 or later on that computer, and its administrator's password.

### 3.1 Install PostgreSQL and pgvector on the server computer

Pick the way that fits the computer:

- **Linux (Debian, Ubuntu)**: PostgreSQL's own packages bring pgvector:
  ```
  sudo apt install -y postgresql-common
  sudo /usr/share/postgresql-common/pgdg/apt.postgresql.org.sh
  sudo apt install -y postgresql-17 postgresql-17-pgvector
  sudo -u postgres psql -c "ALTER USER postgres PASSWORD 'a long password of your own'"
  ```
- **Docker**, on any system or on a NAS with a container manager: the image `pgvector/pgvector:pg17` is PostgreSQL
  with pgvector. Give it a data volume, publish port 5432, and set `POSTGRES_PASSWORD` to a long password of your own.
- **macOS**: with Homebrew, `brew install postgresql@17 pgvector`, then `brew services start postgresql@17`, and
  give the `postgres` login a password (`psql postgres -c "CREATE ROLE postgres LOGIN SUPERUSER PASSWORD '...'"`
  where Homebrew made your own user the administrator).
- **Windows**: pgvector has no Windows installer of its own. Two ways work:
  - From conda-forge, without building anything (tried on a fresh Windows 11). In PowerShell as an administrator,
    with a long password of your own in place of `a long password of your own`:
    ```
    # Microsoft's Visual C++ runtime, which these programs need and a fresh Windows doesn't have
    Invoke-WebRequest https://aka.ms/vs/17/release/vc_redist.x64.exe -OutFile $env:TEMP\vc_redist.x64.exe
    Start-Process $env:TEMP\vc_redist.x64.exe -ArgumentList /install, /quiet, /norestart -Wait
    # PostgreSQL and pgvector into C:\pg
    New-Item -ItemType Directory C:\pg
    Invoke-WebRequest https://github.com/mamba-org/micromamba-releases/releases/download/2.0.5-0/micromamba-win-64 -OutFile C:\pg\micromamba.exe
    $env:MAMBA_ROOT_PREFIX = "C:\pg\mamba"
    C:\pg\micromamba.exe create -y -p C:\pg\env -c conda-forge postgresql pgvector
    # the data folder, with the postgres login's password
    Set-Content C:\pg\pw.txt 'a long password of your own' -NoNewline
    C:\pg\env\Library\bin\initdb.exe -D C:\pg\data -U postgres -A scram-sha-256 --pwfile=C:\pg\pw.txt -E UTF8 --locale=C
    Remove-Item C:\pg\pw.txt
    ```
    Then part 3.2 and part 3.3 (the data folder is `C:\pg\data`), and start it as a Windows service that starts
    with Windows:
    ```
    icacls C:\pg\data /grant "NT AUTHORITY\NetworkService:(OI)(CI)M" /T /Q
    C:\pg\env\Library\bin\pg_ctl.exe register -N facerec-postgres -D C:\pg\data -S auto
    Start-Service facerec-postgres
    ```
  - PostgreSQL's installer from postgresql.org (EDB), which asks for the `postgres` password and makes the
    service, with pgvector built as its README says (Visual Studio's C++ build tools, then `nmake /F Makefile.win`
    and `nmake /F Makefile.win install`). Docker Desktop, as above, works too.

### 3.2 Make the certificates in the app

On any device that will use the database (it doesn't have to be the server):

1. Settings, **Devices and shared database**, **Certificates for a new database server**.
2. Type the server's address as your devices reach it (for example `192.168.1.20`), and click **Make them**.
3. The app makes a certificate authority of yours (kept on this device, its key protected) and, under it, the
   server's certificate and key. It puts them in a folder it names (`database-server` in the app's private folder,
   Settings, App, **Open folder**) with `setup-the-server.txt`, the same steps as below.

Keep that device: a server's certificate made again later is made under the same authority, so no device has to
join again.

### 3.3 Set up the server

1. Copy `server.crt` and `server.key` from that folder into the server's data folder, the folder that holds
   `postgresql.conf`:
   - Linux (Debian, Ubuntu): `/etc/postgresql/17/main/` holds the settings, the data folder is
     `/var/lib/postgresql/17/main/`: put the two files there, then `sudo chown postgres:postgres server.*` and
     `sudo chmod 600 server.key`.
   - Windows: `C:\Program Files\PostgreSQL\17\data\` with EDB's installer, `C:\pg\data` the conda-forge way.
   - macOS (Homebrew): `/opt/homebrew/var/postgresql@17/`, then `chmod 600 server.key`.
   - Docker: put them in the data volume, owned by the container's postgres user with `chmod 600` on the key.
2. In `postgresql.conf`, set these (remove a `#` at the start of a line to use it):
   ```
   listen_addresses = '*'
   ssl = on
   ssl_cert_file = 'server.crt'
   ssl_key_file = 'server.key'
   password_encryption = scram-sha-256
   ```
3. At the end of `pg_hba.conf` (beside `postgresql.conf`), add a line for your local network, with your own
   network's range in place of `192.168.1.0/24`:
   ```
   hostssl  all  all  192.168.1.0/24  scram-sha-256
   ```
4. Restart PostgreSQL (Linux `sudo systemctl restart postgresql`; Windows, Services, the postgresql service
   (`facerec-postgres` the conda-forge way), Restart; macOS `brew services restart postgresql@17`; Docker, restart the container).
5. Let port 5432 through the server's firewall, for your local network only.

### 3.4 Connect

1. Settings, **Devices and shared database**, **Shared database**, **Connect**: the server's address, port 5432,
   database `facerec`, user **postgres** and its password.
2. The app checks that the server shows the certificate it made, then prepares everything itself: its own login
   (`facerec`, not an administrator, with a password of its own making), the database, pgvector in it, its tables,
   and your certificate authority in the server for every device to take. It keeps only its own login on this
   device, never the administrator's password.
3. It says **Connected** with PostgreSQL's and pgvector's versions. This device's library goes to the database in
   the background.

If Connect says something else, it says what to do; the usual ones:

| It says | Do |
|---|---|
| the server doesn't show a certificate from this device's certificate authority | Step 3.3: the two files in the data folder, the lines in `postgresql.conf`, a restart. |
| can't be reached / TLS handshake | The server's address, the port, `listen_addresses`, the firewall; `ssl = on`. |
| refused this device's login | The password; the `hostssl` line in `pg_hba.conf` covers this device's address. |
| pgvector isn't installed on the server | Part 3.1's pgvector, then Connect again. |
| this server is set up for Face Recognizer already | The server has a setup: use part 4 (join from one of your devices), or its own login. |

### 3.5 The passphrase for keys (recommended)

Settings, **Passphrase for keys**: type a passphrase of at least 10 characters twice. Your Web search keys and the
model server's key are then kept in the database encrypted with it, and every device you let in gets it from you.

---

## 4. Each further device

A device joins without typing anything: one device shows a code, the other reads it with its camera.

1. Both devices on the same local network (not a guest Wi-Fi, which keeps devices apart).
2. On the new device: install and start the app (parts 1 and 2), then Face tracking, Controls, **Join a shared
   database**: it shows a code. On a device that has the database: Face tracking, Controls, **Join a new device**,
   scan it with the camera, and say yes. (Or the other way round: the device that has it shows a code, and the new
   one scans it; a phone's own camera app reads it too.)
3. The new device gets the login and the passphrase, encrypted, and is in. Every device's alerts say a device joined.

The new device itself must reach the database server (part 3.3's `hostssl` line covers your whole network).

---

## 5. Describing photos and Analyze

**Built-in model** (photos only, on the processor): Settings, **AI models**, **Describe photo**, **Built-in model**,
Download (0.8 GB). Nothing else is needed; Describe photo uses it when no model server answers.

**A model server** gives better descriptions and is needed for Analyze. Install one of these, on this computer or
another of yours (Settings, AI models, **Model server**, **Suggested for this computer** names a model that fits
this computer's graphics memory, and **Load it** loads it):

- **LM Studio** (lmstudio.ai): install, download a vision model in it (for example Qwen 3.6 35B-A3B or Gemma 4),
  and start its server (Developer tab). Kind of server: LM Studio.
- **Ollama** (ollama.com): install, then `ollama pull qwen3.6:35b-a3b` (or another vision model). Kind: Ollama.
- **llama.cpp**: `llama-server -m model.gguf --mmproj mmproj.gguf --host 0.0.0.0 --port 8080`. Kind: llama.cpp.
- **vLLM** (Linux with an NVIDIA card): `vllm serve <model> --host 0.0.0.0`. Kind: vLLM.

Then in Settings, AI models:

1. **Model server**: the kind, and the address (empty for this computer). **Test** says which models it has.
2. **Analyze**: **Where Analyze runs** lists your devices' servers; another computer keeps this one's processor
   for its camera. **Load the model for Analyze** loads the model with the longer context Analyze needs.
3. **Model servers on this computer**, **Keep the model servers here open to my other devices**: switch it on to
   let your other devices use this computer's servers. The app then keeps LM Studio serving on your network, and
   starts Ollama, llama.cpp or vLLM again listening on your network where they listen on this computer only. Off
   for a new install: until you switch it on, the app changes nothing of theirs.
4. Analyze's microphone: Analyze, Settings, **Get the speech model** (78 MB), once.

---

## 6. Optional services

**Web search on a face** (Labeling, Web search): your own keys, kept on your devices.

- Google: in the Google Cloud console, make a project, give it billing, enable the **Cloud Vision API**, and make
  an **API key** restricted to that API. Settings, **Web search**, **Google Cloud Vision API key**.
- Amazon: in AWS, make an IAM user allowed only `rekognition:RecognizeCelebrities`, and an access key for it.
  Settings, **Amazon Rekognition key**: its ID, secret and region (for example `us-east-1`).

**Music** (Spotify): at developer.spotify.com/dashboard, create an app, add the redirect address
`http://127.0.0.1:8768/callback` (the address Settings shows), tick Web API, and copy its Client ID into Settings,
**Music**, **Spotify**, Connect. The app asks only to read your library (`user-library-read`).

**Location**: Settings, **Location**, switch it on; the system asks once (Windows: Settings, Privacy & security,
Location, allow desktop apps). Linux and a Raspberry Pi have no location service: type the place.

---

## 7. Your network and firewall

The app's own pages listen on this computer only. With a shared database, these reach the network:

| Port | What | Where |
|---|---|---|
| 8767 | photos and live cameras between your devices (encrypted, your devices only) | every device that shares |
| 8770 | a join code, for two minutes while one is shown | the device that shows a code |
| 5432 | the database | the server |
| 1234, 11434, 8080, 8000 | LM Studio, Ollama, llama.cpp, vLLM | a computer whose model server your devices use |

On Windows, set your network as Private first (Windows Settings, Network & internet, your network, Private
network): on a Public one Windows lets nothing in. The first time a program listens on your network (Face
Recognizer once it is connected to a shared database), Windows asks whether to "allow public and private networks
to access this app": choose **Show more**, untick **Public networks**, tick **Private networks**, then **Allow**
(an administrator's password may be asked). Allow alone opens it to the kind of network you are on now, which may
be Public. On a Mac,
allow incoming connections when macOS asks.
