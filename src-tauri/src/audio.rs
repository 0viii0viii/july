//! 마이크 녹음.
//!
//! cpal의 `Stream`은 `Send`가 아니라서 커맨드 사이를 넘나들 수 없다. 그래서
//! 스트림을 소유하는 전용 스레드를 하나 띄우고, 바깥과는 원자 변수와 채널로만
//! 이야기한다.
//!
//! 출력은 whisper가 요구하는 16kHz 모노 16비트 WAV로 바로 떨군다 — 변환
//! 단계를 하나 없애서 녹음이 끝나면 곧장 전사할 수 있게 한다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 레벨 미터가 이 값 아래로 계속 머무르면 마이크가 실제로 안 잡히는 것으로
/// 본다. 사용자가 30분을 녹음하고 나서야 무음이었다는 걸 아는 상황을 막는다.
const SILENCE_THRESHOLD: f32 = 0.002;

#[derive(Debug, Clone, Serialize)]
pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingStatus {
    pub recording: bool,
    /// 0.0 ~ 1.0. 최근 버퍼의 피크. 레벨 미터용.
    pub level: f32,
    /// 녹음 경과 시간(초).
    pub seconds: f64,
    /// 시작 후 계속 무음이면 true. 마이크 권한이나 입력 선택 문제일 수 있다.
    pub silent: bool,
}

/// 녹음 스레드와 주고받는 공유 상태.
struct Shared {
    stop: AtomicBool,
    /// f32 피크를 비트 패턴으로 저장한다 (AtomicF32이 없어서).
    level_bits: AtomicU32,
    samples_written: AtomicUsize,
    /// 유의미한 소리가 한 번이라도 잡혔는지.
    heard_sound: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            level_bits: AtomicU32::new(0),
            samples_written: AtomicUsize::new(0),
            heard_sound: AtomicBool::new(false),
        }
    }

    fn level(&self) -> f32 {
        f32::from_bits(self.level_bits.load(Ordering::Relaxed))
    }

    fn set_level(&self, v: f32) {
        self.level_bits.store(v.to_bits(), Ordering::Relaxed);
    }
}

pub struct Recorder {
    active: Mutex<Option<Active>>,
}

struct Active {
    shared: Arc<Shared>,
    path: PathBuf,
    /// 녹음 스레드가 끝나면서 결과를 돌려준다.
    done: mpsc::Receiver<Result<(), String>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active.lock().map(|a| a.is_some()).unwrap_or(false)
    }

    pub fn status(&self) -> RecordingStatus {
        let guard = self.active.lock().expect("녹음 상태 잠금 실패");
        match guard.as_ref() {
            None => RecordingStatus {
                recording: false,
                level: 0.0,
                seconds: 0.0,
                silent: false,
            },
            Some(active) => {
                let written = active.shared.samples_written.load(Ordering::Relaxed);
                let seconds = written as f64 / TARGET_SAMPLE_RATE as f64;
                RecordingStatus {
                    recording: true,
                    level: active.shared.level(),
                    seconds,
                    // 3초는 지나야 판단한다 — 시작 직후의 정적은 정상이다.
                    silent: seconds > 3.0
                        && !active.shared.heard_sound.load(Ordering::Relaxed),
                }
            }
        }
    }

    /// 녹음을 시작하고 결과 파일 경로를 돌려준다.
    pub fn start(&self, dest: &Path, wanted_device: Option<&str>) -> Result<PathBuf, String> {
        let mut guard = self.active.lock().expect("녹음 상태 잠금 실패");
        if guard.is_some() {
            return Err("이미 녹음 중입니다.".into());
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("저장 폴더를 만들 수 없습니다: {e}"))?;
        }

        let shared = Arc::new(Shared::new());
        let (done_tx, done_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let path = dest.to_path_buf();
        let thread_path = path.clone();
        let thread_shared = Arc::clone(&shared);
        let wanted_device = wanted_device.map(str::to_owned);

        std::thread::spawn(move || {
            let result = record_loop(
                &thread_path,
                wanted_device.as_deref(),
                &thread_shared,
                &ready_tx,
            );
            // 시작 단계에서 실패했다면 ready 채널로 이미 알렸다. 여기서는
            // 최종 결과만 전달한다.
            let _ = done_tx.send(result);
        });

        // 스트림이 실제로 열릴 때까지 기다린다. 장치 없음·권한 거부 같은
        // 실패를 이 시점에 사용자에게 알려야 한다.
        match ready_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("녹음 장치를 여는 데 시간이 너무 오래 걸립니다.".into()),
        }

        *guard = Some(Active {
            shared,
            path: path.clone(),
            done: done_rx,
        });
        Ok(path)
    }

    /// 녹음을 멈추고 저장된 파일 경로를 돌려준다.
    pub fn stop(&self) -> Result<PathBuf, String> {
        let mut guard = self.active.lock().expect("녹음 상태 잠금 실패");
        let active = guard.take().ok_or("녹음 중이 아닙니다.")?;

        active.shared.stop.store(true, Ordering::Relaxed);
        // WAV 헤더를 확정해야 하므로 스레드가 끝날 때까지 기다린다.
        match active.done.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(active.path),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("녹음을 정리하는 데 시간이 너무 오래 걸립니다.".into()),
        }
    }
}

/// 장치 이름을 안전하게 읽는다.
///
/// cpal 0.18에는 `name()`이 없고 `Display`로 이름을 얻게 돼 있는데, 그 구현이
/// `description()` 실패 시 `fmt::Error`를 돌려준다. `to_string()`은 이걸
/// **패닉으로 바꾼다** — 장치 하나가 잠깐 사라지기만 해도 앱 전체가 죽는다.
/// 그래서 `description()`을 직접 부르고 실패는 None으로 흘린다.
fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

/// 사용 가능한 입력 장치 목록.
pub fn input_devices() -> Result<Vec<InputDevice>, String> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().as_ref().and_then(device_name);

    let devices = host
        .input_devices()
        .map_err(|e| format!("입력 장치를 조회할 수 없습니다: {e}"))?;

    Ok(devices
        // 이름을 못 읽는 장치는 사용자가 고를 수도 없으니 조용히 건너뛴다.
        .filter_map(|d| device_name(&d))
        .map(|name| InputDevice {
            is_default: Some(&name) == default_name.as_ref(),
            name,
        })
        .collect())
}

/// 실제 녹음 루프. 전용 스레드에서만 돈다.
fn record_loop(
    path: &Path,
    wanted_device: Option<&str>,
    shared: &Arc<Shared>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    // 시작 단계 실패는 ready 채널로 알린 뒤 그대로 종료한다.
    macro_rules! bail_start {
        ($e:expr) => {{
            let err: String = $e;
            let _ = ready.send(Err(err.clone()));
            return Err(err);
        }};
    }

    let host = cpal::default_host();
    let device = match wanted_device {
        Some(wanted) => host
            .input_devices()
            .ok()
            .and_then(|mut ds| {
                ds.find(|d| device_name(d).as_deref() == Some(wanted))
            }),
        None => host.default_input_device(),
    };

    let Some(device) = device else {
        bail_start!("사용할 수 있는 마이크를 찾지 못했습니다.".to_string());
    };

    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => bail_start!(format!(
            "마이크 설정을 읽을 수 없습니다. 시스템 설정 > 개인정보 보호 및 \
             보안 > 마이크에서 접근을 허용했는지 확인해 주세요: {e}"
        )),
    };

    // cpal 0.18에서 SampleRate는 u32 별칭이다 (예전의 튜플 구조체가 아니다).
    let source_rate = config.sample_rate();
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = match hound::WavWriter::create(path, spec) {
        Ok(w) => w,
        Err(e) => bail_start!(format!("녹음 파일을 만들 수 없습니다: {e}")),
    };
    let writer = Arc::new(Mutex::new(Some(writer)));

    let cb_writer = Arc::clone(&writer);
    let cb_shared = Arc::clone(shared);
    // 리샘플링 위상. 콜백 사이에 이어져야 해서 클로저가 소유한다.
    let mut phase = 0.0f64;
    let step = source_rate as f64 / TARGET_SAMPLE_RATE as f64;

    let mut on_samples = move |input: &[f32]| {
        if input.is_empty() {
            return;
        }

        // 멀티채널은 평균으로 모노화한다.
        let frames: Vec<f32> = if channels <= 1 {
            input.to_vec()
        } else {
            input
                .chunks(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32)
                .collect()
        };

        let peak = frames.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        cb_shared.set_level(peak);
        if peak > SILENCE_THRESHOLD {
            cb_shared.heard_sound.store(true, Ordering::Relaxed);
        }

        // 선형 보간 리샘플링. 음성 대역에는 충분하고, 오디오 콜백 안에서
        // 돌려야 하므로 가벼워야 한다.
        let mut out = Vec::with_capacity((frames.len() as f64 / step) as usize + 2);
        while phase < frames.len() as f64 {
            let i = phase as usize;
            let frac = (phase - i as f64) as f32;
            let a = frames[i];
            let b = *frames.get(i + 1).unwrap_or(&a);
            let v = a + (b - a) * frac;
            out.push((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
            phase += step;
        }
        phase -= frames.len() as f64;

        if let Ok(mut guard) = cb_writer.lock() {
            if let Some(w) = guard.as_mut() {
                for s in &out {
                    // 개별 샘플 실패로 녹음 전체를 중단시키지는 않는다.
                    let _ = w.write_sample(*s);
                }
            }
        }
        cb_shared
            .samples_written
            .fetch_add(out.len(), Ordering::Relaxed);
    };

    let err_fn = |e| eprintln!("녹음 스트림 오류: {e}");
    // 0.18부터 build_input_stream이 config를 값으로 받는다.
    let stream_config: cpal::StreamConfig = config.clone().into();

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            stream_config,
            move |data: &[f32], _: &_| on_samples(data),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            stream_config,
            move |data: &[i16], _: &_| {
                let f: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                on_samples(&f)
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            stream_config,
            move |data: &[u16], _: &_| {
                let f: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 - 32768.0) / 32768.0)
                    .collect();
                on_samples(&f)
            },
            err_fn,
            None,
        ),
        other => bail_start!(format!("지원하지 않는 오디오 형식입니다: {other:?}")),
    };

    let stream = match stream {
        Ok(s) => s,
        Err(e) => bail_start!(format!(
            "마이크를 열 수 없습니다. 다른 앱이 사용 중이거나 권한이 없을 수 \
             있습니다: {e}"
        )),
    };

    if let Err(e) = stream.play() {
        bail_start!(format!("녹음을 시작할 수 없습니다: {e}"));
    }

    let _ = ready.send(Ok(()));

    while !shared.stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    drop(stream);

    // WAV 헤더의 길이 필드는 finalize에서 확정된다. 이걸 빠뜨리면 파일이
    // 깨진 것처럼 보인다.
    let writer = writer
        .lock()
        .map_err(|_| "녹음 파일 잠금에 실패했습니다.".to_string())?
        .take()
        .ok_or("녹음 파일이 이미 닫혔습니다.")?;
    writer
        .finalize()
        .map_err(|e| format!("녹음 파일을 저장할 수 없습니다: {e}"))?;

    Ok(())
}
