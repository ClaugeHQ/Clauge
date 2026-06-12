// Adapted from meetily (MIT) — github.com/Zackriya-Solutions/meetily

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use cidre::core_audio::{self as ca, aggregate_device_keys as agg_keys};
use cidre::{cat, cf, ns, os};

use super::SystemAudioError;
use crate::shared::audio::{AudioFrame, CaptureEvent};
use crate::shared::platform::macos::macos_version;

const MIN_MACOS: (u32, u32) = (14, 4);

pub struct MacSystemCapture {
    stop_tx: Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl MacSystemCapture {
    /// Starts a global Core Audio process tap (system output mix-down).
    ///
    /// The first call triggers macOS's system-audio-capture permission dialog.
    /// If the user denies permission, Core Audio does NOT return an error —
    /// the tap simply delivers silence (all-zero samples). Callers cannot
    /// distinguish "denied" from "nothing is playing" at this layer.
    pub fn start(tx: Sender<CaptureEvent>) -> Result<Self, SystemAudioError> {
        let version = macos_version().ok_or_else(|| {
            SystemAudioError::Unsupported("could not determine macOS version".to_string())
        })?;
        if version < MIN_MACOS {
            return Err(SystemAudioError::Unsupported(
                "macOS 14.4+ required for system audio capture".to_string(),
            ));
        }

        let (stop_tx, stop_rx) = channel();
        let (ready_tx, ready_rx) = channel();
        let handle = std::thread::spawn(move || run_capture(tx, stop_rx, ready_tx));
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop_tx,
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err(SystemAudioError::Failed(
                    "system capture thread exited before reporting status".to_string(),
                ))
            }
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MacSystemCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ProcCtx {
    tx: Sender<CaptureEvent>,
    channels: u16,
    rate: u32,
}

/// Owns the live Core Audio objects. Field order is the teardown order:
/// dropping `started` stops IO and destroys the aggregate device (which
/// releases its IO procs), `ctx` is freed only after IO has stopped, and
/// the tap is destroyed last.
struct TapSession {
    started: ca::hardware::StartedDevice<ca::AggregateDevice>,
    ctx: Box<ProcCtx>,
    tap: ca::TapGuard,
}

fn run_capture(
    tx: Sender<CaptureEvent>,
    stop_rx: Receiver<()>,
    ready_tx: Sender<Result<(), SystemAudioError>>,
) {
    let session = match open_tap_session(tx) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let _ = ready_tx.send(Ok(()));
    let _ = stop_rx.recv();
    drop(session);
}

fn open_tap_session(tx: Sender<CaptureEvent>) -> Result<TapSession, SystemAudioError> {
    fn failed(what: &'static str) -> impl Fn(os::Error) -> SystemAudioError {
        move |e| SystemAudioError::Failed(format!("{what}: {e:?}"))
    }

    let output_device =
        ca::System::default_output_device().map_err(failed("get default output device"))?;
    let output_uid = output_device.uid().map_err(failed("get output device UID"))?;

    // Mono global tap: one mixed-down channel of everything routed to the
    // default output. Mono halves the data and is all transcription needs.
    let tap_desc = ca::TapDesc::with_mono_global_tap_excluding_processes(&ns::Array::new());
    let tap = tap_desc
        .create_process_tap()
        .map_err(failed("create process tap"))?;
    let asbd = tap.asbd().map_err(failed("get tap format"))?;
    let tap_uid = tap.uid().map_err(failed("get tap UID"))?;

    let sub_tap = cf::DictionaryOf::with_keys_values(
        &[ca::hardware::sub_tap_keys::uid()],
        &[tap_uid.as_type_ref()],
    );

    // Aggregate device contains ONLY the tap — adding the output device as a
    // sub-device too would capture the same audio twice (echo).
    let agg_desc = cf::DictionaryOf::with_keys_values(
        &[
            agg_keys::is_private(),
            agg_keys::is_stacked(),
            agg_keys::tap_auto_start(),
            agg_keys::name(),
            agg_keys::main_sub_device(),
            agg_keys::uid(),
            agg_keys::tap_list(),
        ],
        &[
            cf::Boolean::value_true().as_type_ref(),
            cf::Boolean::value_false(),
            cf::Boolean::value_true(),
            cf::str!(c"clauge-system-audio-tap").as_type_ref(),
            &output_uid,
            &cf::Uuid::new().to_cf_string(),
            &cf::ArrayOf::from_slice(&[sub_tap.as_ref()]),
        ],
    );

    let agg_device =
        ca::AggregateDevice::with_desc(&agg_desc).map_err(failed("create aggregate device"))?;

    let mut ctx = Box::new(ProcCtx {
        tx,
        channels: asbd.channels_per_frame.max(1) as u16,
        rate: asbd.sample_rate as u32,
    });

    let proc_id = agg_device
        .create_io_proc_id(audio_proc, Some(&mut *ctx))
        .map_err(failed("create IO proc"))?;
    let started =
        ca::hardware::device_start(agg_device, Some(proc_id)).map_err(failed("start device"))?;

    Ok(TapSession { started, ctx, tap })
}

extern "C" fn audio_proc(
    _device: ca::Device,
    _now: &cat::AudioTimeStamp,
    input_data: &cat::AudioBufList<1>,
    _input_time: &cat::AudioTimeStamp,
    _output_data: &mut cat::AudioBufList<1>,
    _output_time: &cat::AudioTimeStamp,
    ctx: Option<&mut ProcCtx>,
) -> os::Status {
    let Some(ctx) = ctx else {
        return os::Status::NO_ERR;
    };
    // The tap delivers native-endian f32 PCM in the first buffer.
    let buf = &input_data.buffers[0];
    let count = buf.data_bytes_size as usize / std::mem::size_of::<f32>();
    if count > 0 && !buf.data.is_null() {
        let data = unsafe { std::slice::from_raw_parts(buf.data as *const f32, count) };
        let _ = ctx.tx.send(CaptureEvent::Frame(AudioFrame {
            samples: data.to_vec(),
            channels: ctx.channels,
            rate: ctx.rate,
        }));
    }
    os::Status::NO_ERR
}
