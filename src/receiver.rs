use bytemuck::{Pod, Zeroable, cast_slice};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use crossbeam_channel::bounded;
use std::collections::VecDeque;
use std::io::Read;
use std::net::TcpListener;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH}; // Add this import

const FRAME_SIZE: usize = 1024; // This is the number of *samples* per frame

#[derive(Clone, Copy)]
#[repr(C)]
struct Sample(f32);

pub fn receiver_stream() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No output device available");

    // Explicitly choose the same sample rate as the sender
    let sample_rate = cpal::SampleRate(44100); // Or 48000, must match sender
    let config = StreamConfig {
        channels: 2, // Assuming stereo, adjust if mono
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    println!("Using output device: {}", device.name()?);
    println!("Using sample rate: {:?}", config.sample_rate);
    println!("Using channels: {:?}", config.channels);

    // Channel to send audio data from network thread to audio callback
    let (tx, rx) = bounded::<Vec<f32>>(32);

    // === Spawn TCP server thread ===
    thread::spawn(move || {
        let listener = TcpListener::bind("192.168.15.21:7777").expect("Failed to bind TCP socket");
        println!("Listening for audio on 192.168.15.21:7777...");

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    println!("Client connected: {:?}", stream.peer_addr());

                    loop {
                        // First, read the size of the incoming chunk
                        let mut size_buf = [0u8; 4]; // u32 is 4 bytes
                        if let Err(err) = stream.read_exact(&mut size_buf) {
                            eprintln!("Failed to read chunk size: {:?}", err);
                            break;
                        }
                        let chunk_size = u32::from_le_bytes(size_buf) as usize;

                        let mut ts_buf = [0u8; 8];
                        if let Err(err) = stream.read_exact(&mut ts_buf) {
                            eprintln!("Failed to read timestamp: {:?}", err);
                            break;
                        }

                        let timestamp = u64::from_le_bytes(ts_buf);

                        // Then, read the audio data itself
                        let mut buf = vec![0u8; chunk_size];
                        if let Err(err) = stream.read_exact(&mut buf) {
                            eprintln!("Read error: {:?}", err);
                            break;
                        }

                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                        let now_ms = now.as_millis() as u64;
                        let latency = now_ms.saturating_sub(timestamp);

                        if latency != 0 {
                            println!("Latency: {} ms ", latency);
                        }

                        let samples: &[f32] = cast_slice(&buf);
                        if tx.send(samples.to_vec()).is_err() {
                            break;
                        }
                    }

                    println!("Client disconnected.");
                }
                Err(err) => eprintln!("Connection failed: {:?}", err),
            }
        }
    });

    let mut buffer: VecDeque<f32> = VecDeque::new();
    // === Start CPAL output stream ===
    let stream = device.build_output_stream(
        &config,
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // Corrected callback signature
            // Pull samples from the network into the buffer
            while let Ok(mut samples) = rx.try_recv() {
                buffer.extend(samples.drain(..));
            }

            // Write from buffer to output, fill with silence if needed
            for o in output.iter_mut() {
                if let Some(sample) = buffer.pop_front() {
                    *o = sample;
                } else {
                    *o = 0.0; // underrun - no data
                }
            }
        },
        move |err| {
            eprintln!("CPAL error: {:?}", err);
        },
        None,
    )?;

    stream.play()?;
    println!("Audio stream started.");

    // Keep alive
    loop {
        std::thread::park();
    }
}
