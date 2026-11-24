//! Listening to this computer's audio interfaces (`interfaces`, `record` and
//! `listen_all`): every output, heard exactly as the computer plays it (loopback), and every microphone,
//! which hears the room and whatever plays in it from any device. Each is listened to at the same moment,
//! for a few seconds, and only numbers leave the listening: the sound is kept in memory while it is
//! measured and never written anywhere.
//!
//! The sound comes through the `cpal` library, built into the program: WASAPI on Windows (an output is
//! listened to by its own loopback), CoreAudio on macOS (an output by a tap on what the system plays,
//! macOS 14.2 and later), and on Linux the sound server every desktop has (PulseAudio's protocol, which
//! PipeWire speaks too: an output by its monitor), else ALSA with the microphones alone.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use serde_json::Value;

use crate::sound::{self, RATE, Sounds};

/// One audio interface that can be listened to.
pub struct Interface {
    pub id: String,
    pub name: String,
    /// "output": what the computer itself plays through it; "input": what a microphone hears
    pub kind: &'static str,
    pub default: bool,
    /// the device the sound is taken from (an output's own loopback, or on Linux its monitor)
    from: cpal::Device,
}

/// The sound system to ask: on Linux the sound server when it answers (its outputs can be listened to),
/// elsewhere the system's own.
fn host() -> cpal::Host {
    #[cfg(target_os = "linux")]
    if let Ok(server) = cpal::host_from_id(cpal::HostId::PulseAudio) {
        return server;
    }
    cpal::default_host()
}

fn id_of(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn name_of(device: &cpal::Device) -> String {
    let name = device.description().map(|d| d.name().to_string()).ok().or_else(|| id_of(device)).unwrap_or_default();
    name.chars().take(120).collect()
}

/// Whether a microphone's name says it is an iPhone's or an iPad's (macOS names one after the phone).
pub fn named_as_a_phone(name: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("iphone") || name.contains("ipad")
}

/// The microphones macOS offers from an iPhone or iPad (Continuity), by their own ids: what the system itself
/// says of each audio device's transport ("ccwd", "ccwl": Continuity capture, wired and wireless). Empty when
/// the system can't be asked.
#[cfg(target_os = "macos")]
fn phone_microphones() -> Vec<String> {
    use std::ptr::{NonNull, null};

    use objc2_core_audio::{AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectPropertyAddress};
    use objc2_core_foundation::{CFRetained, CFString};

    const fn code(letters: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*letters)
    }
    // (the system's own object, and a property of an object as a whole: "glob", element 0)
    const SYSTEM: u32 = 1;
    let whole = |selector: &[u8; 4]| AudioObjectPropertyAddress {
        mSelector: code(selector),
        mScope: code(b"glob"),
        mElement: 0,
    };
    let (every, mut size) = (whole(b"dev#"), 0u32);
    // SAFETY: the system writes the size of its list of devices into `size`; the status is looked at.
    if unsafe { AudioObjectGetPropertyDataSize(SYSTEM, NonNull::from(&every), 0, null(), NonNull::from(&mut size)) }
        != 0
    {
        return Vec::new();
    }
    let mut devices = vec![0u32; size as usize / 4];
    let Some(list) = NonNull::new(devices.as_mut_ptr()) else { return Vec::new() };
    // SAFETY: the list has room for `size` bytes of device numbers, which is what the system writes.
    if devices.is_empty()
        || unsafe {
            AudioObjectGetPropertyData(SYSTEM, NonNull::from(&every), 0, null(), NonNull::from(&mut size), list.cast())
        } != 0
    {
        return Vec::new();
    }
    devices.truncate(size as usize / 4);
    let mut phones = Vec::new();
    for device in devices {
        let (transport_of, mut transport, mut four) = (whole(b"tran"), 0u32, 4u32);
        // SAFETY: a device's transport is one 32-bit number, written into `transport`.
        let asked = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&transport_of),
                0,
                null(),
                NonNull::from(&mut four),
                NonNull::from(&mut transport).cast(),
            )
        };
        if asked != 0 || !(transport == code(b"ccwd") || transport == code(b"ccwl")) {
            continue;
        }
        let (id_of, mut id, mut pointer) =
            (whole(b"uid "), std::ptr::null_mut::<CFString>(), size_of::<*mut CFString>() as u32);
        // SAFETY: a device's id is one text the system makes for the asker, who lets go of it (CFRetained).
        let asked = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&id_of),
                0,
                null(),
                NonNull::from(&mut pointer),
                NonNull::from(&mut id).cast(),
            )
        };
        if let (0, Some(id)) = (asked, NonNull::new(id)) {
            // SAFETY: the text was made for this call and is this call's to let go of.
            phones.push(unsafe { CFRetained::from_raw(id) }.to_string());
        }
    }
    phones
}

/// Every audio interface of this computer that can be listened to, and why some couldn't be listed.
///
/// A Mac leaves out the microphone of an iPhone or iPad (macOS offers a nearby one as a microphone of the Mac):
/// the owner's decision, 2026-11-22, "I want the iOS app to listen to sounds separately from the macOS device
/// even when they are connected". Listening to it takes the phone over (it shows "Pause or Disconnect" and its
/// own app stops), so the Mac never opens it: told by the transport the system names for the device, and by
/// its name where the system wasn't asked.
pub fn interfaces() -> (Vec<Interface>, Vec<String>) {
    let (host, mut found, mut why) = (host(), Vec::new(), Vec::new());
    let devices: Vec<cpal::Device> = match host.devices() {
        Ok(devices) => devices.collect(),
        Err(e) => {
            why.push(format!("the audio interfaces couldn't be listed ({e})"));
            return (found, why);
        }
    };
    // (on Linux an output's sound is its monitor's, which the sound server lists among the inputs)
    let monitor = |id: &str| id.ends_with(".monitor");
    let (first_out, first_in) =
        (host.default_output_device().as_ref().and_then(id_of), host.default_input_device().as_ref().and_then(id_of));
    // each kind by itself: a computer without speakers still has its microphones
    for device in devices.iter().filter(|d| d.supports_output()) {
        let Some(id) = id_of(device) else { continue };
        let from = if cfg!(target_os = "linux") {
            devices.iter().find(|d| id_of(d).as_deref() == Some(&format!("{id}.monitor"))).cloned()
        } else {
            Some(device.clone())
        };
        match from {
            Some(from) => found.push(Interface {
                default: Some(&id) == first_out.as_ref(),
                name: name_of(device),
                kind: "output",
                id,
                from,
            }),
            None if !why.iter().any(|w: &String| w.starts_with("what this computer plays")) => {
                why.push(
                    "what this computer plays can't be listened to here (no sound server that offers it)".to_string(),
                );
            }
            None => {}
        }
    }
    #[cfg(target_os = "macos")]
    let phones = phone_microphones();
    for device in devices.iter().filter(|d| d.supports_input()) {
        let Some(id) = id_of(device).filter(|id| !monitor(id)) else { continue };
        #[cfg(target_os = "macos")]
        if phones.iter().any(|phone| id.contains(phone.as_str())) || named_as_a_phone(&name_of(device)) {
            if !why.iter().any(|w: &String| w.starts_with("an iPhone or iPad")) {
                why.push(
                    "an iPhone or iPad offered as a microphone is left to the phone: it listens for itself".to_string(),
                );
            }
            continue;
        }
        found.push(Interface {
            default: Some(&id) == first_in.as_ref(),
            name: name_of(device),
            kind: "input",
            id,
            from: device.clone(),
        });
    }
    if found.is_empty() && why.is_empty() {
        why.push("this computer has no audio interface the program can see".to_string());
    }
    (found, why)
}

/// What one interface plays or hears for `seconds`, as one channel of numbers at `RATE` a second, or None
/// when it can't be listened to. An output that plays nothing gives nothing at all on some systems: that is
/// silence, as long as asked for. Kept in memory only.
pub fn record(interface: &Interface, seconds: f64) -> Option<Vec<f32>> {
    let device = &interface.from;
    let config =
        if device.supports_input() { device.default_input_config() } else { device.default_output_config() }.ok()?;
    let (channels, rate, format) =
        (config.channels().max(1) as usize, config.sample_rate() as usize, config.sample_format());
    let heard: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity((rate as f64 * seconds) as usize + rate)));
    // each moment's channels become one number: their mean
    fn take<T: Copy>(heard: &Mutex<Vec<f32>>, data: &[T], channels: usize, as_f32: impl Fn(T) -> f32) {
        let mut heard = heard.lock().unwrap_or_else(|e| e.into_inner());
        heard.extend(
            data.chunks(channels).map(|moment| moment.iter().map(|v| as_f32(*v)).sum::<f32>() / moment.len() as f32),
        );
    }
    let (into, quiet) = (heard.clone(), |_: cpal::Error| {});
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            config.into(),
            move |d: &[f32], _: &_| take(&into, d, channels, |v| v),
            quiet,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config.into(),
            move |d: &[i16], _: &_| take(&into, d, channels, |v| v as f32 / 32768.0),
            quiet,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            config.into(),
            move |d: &[u16], _: &_| take(&into, d, channels, |v| (v as f32 - 32768.0) / 32768.0),
            quiet,
            None,
        ),
        SampleFormat::I32 => device.build_input_stream(
            config.into(),
            move |d: &[i32], _: &_| take(&into, d, channels, |v| v as f32 / 2_147_483_648.0),
            quiet,
            None,
        ),
        SampleFormat::I8 => device.build_input_stream(
            config.into(),
            move |d: &[i8], _: &_| take(&into, d, channels, |v| v as f32 / 128.0),
            quiet,
            None,
        ),
        SampleFormat::U8 => device.build_input_stream(
            config.into(),
            move |d: &[u8], _: &_| take(&into, d, channels, |v| (v as f32 - 128.0) / 128.0),
            quiet,
            None,
        ),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    let (began, wanted) = (Instant::now(), (rate as f64 * seconds) as usize);
    // (a little longer than asked for at most: an interface that sends nothing is not waited for)
    while began.elapsed() < Duration::from_secs_f64(seconds + 1.0)
        && heard.lock().unwrap_or_else(|e| e.into_inner()).len() < wanted
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    let mut x = std::mem::take(&mut *heard.lock().unwrap_or_else(|e| e.into_inner()));
    x.resize(wanted, 0.0);
    Some(sound::resample(&x, rate, RATE))
}

/// Every interface of these kinds listened to at the same moment for `seconds`: [{name, kind, default,
/// heard, sounds, families, ambient, what, sure, background}]. heard: `measure`'s numbers as for a song
/// (none when silent); sounds: the kinds of sound the model hears; ambient: the background's numbers; what
/// and sure: those in a word ("music", "speech", "quiet"...) and how sure. So what plays is known by where
/// it sounded: from this computer through one of its outputs, or in the room, heard by a microphone.
pub fn listen_all(seconds: f64, kinds: &[&str], model: Option<&Sounds>) -> Vec<Value> {
    let (found, _) = interfaces();
    let found: Vec<Interface> = found.into_iter().filter(|i| kinds.contains(&i.kind)).collect();
    let heard: Vec<Option<Vec<f32>>> = std::thread::scope(|scope| {
        let listening: Vec<_> = found.iter().map(|i| scope.spawn(move || record(i, seconds))).collect();
        listening.into_iter().map(|l| l.join().ok().flatten()).collect()
    });
    (found.iter().zip(heard))
        .map(|(i, x)| crate::soundheard::entry(&i.name, i.kind, i.default, x.as_deref(), RATE, model))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_microphone_named_after_an_iphone_or_ipad_is_a_phones() {
        for name in ["Someone's iPhone Microphone", "IPHONE-Mikrofon", "My iPad Microphone", "iphone"] {
            assert!(named_as_a_phone(name), "{name}");
        }
        for name in
            ["MacBook Pro Microphone", "Microphone Array (Realtek(R) Audio)", "USB Microphone", "Phone Link", ""]
        {
            assert!(!named_as_a_phone(name), "{name}");
        }
    }
}
