use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use super::{AudioFrame, CaptureEvent};

pub struct MicCapture {
    stop_tx: Sender<()>,
    handle: JoinHandle<()>,
}

impl MicCapture {
    pub fn start(tx: Sender<CaptureEvent>) -> Result<Self, String> {
        let (stop_tx, stop_rx) = channel();
        let (ready_tx, ready_rx) = channel();
        let handle = std::thread::spawn(move || run_capture(tx, stop_rx, ready_tx));
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { stop_tx, handle }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err("mic capture thread exited before reporting status".to_string())
            }
        }
    }

    pub fn stop(self) {
        let _ = self.stop_tx.send(());
        let _ = self.handle.join();
    }
}

fn run_capture(
    tx: Sender<CaptureEvent>,
    stop_rx: Receiver<()>,
    ready_tx: Sender<Result<(), String>>,
) {
    let stream = match open_input_stream(tx) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready_tx.send(Err(format!("failed to start mic stream: {e}")));
        return;
    }
    let _ = ready_tx.send(Ok(()));
    let _ = stop_rx.recv();
}

fn open_input_stream(tx: Sender<CaptureEvent>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default input device".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;
    let channels = config.channels();
    let rate = config.sample_rate();
    let format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();
    match format {
        SampleFormat::F32 => build_stream::<f32>(&device, stream_config, channels, rate, tx),
        SampleFormat::I16 => build_stream::<i16>(&device, stream_config, channels, rate, tx),
        SampleFormat::U16 => build_stream::<u16>(&device, stream_config, channels, rate, tx),
        SampleFormat::I32 => build_stream::<i32>(&device, stream_config, channels, rate, tx),
        SampleFormat::U32 => build_stream::<u32>(&device, stream_config, channels, rate, tx),
        SampleFormat::F64 => build_stream::<f64>(&device, stream_config, channels, rate, tx),
        other => Err(format!("unsupported mic sample format: {other}")),
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: u16,
    rate: u32,
    tx: Sender<CaptureEvent>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let err_tx = tx.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data.iter().map(|&s| s.to_sample::<f32>()).collect();
                let _ = tx.send(CaptureEvent::Frame(AudioFrame {
                    samples,
                    channels,
                    rate,
                }));
            },
            move |err| {
                let _ = err_tx.send(CaptureEvent::Error(err.to_string()));
            },
            None,
        )
        .map_err(|e| format!("failed to build mic stream: {e}"))
}
