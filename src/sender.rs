use cpal::StreamConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::{io::AsyncWriteExt, net::TcpStream};

const FRAME_SIZE: usize = 1024; // This is the number of *samples* per frame

#[tokio::main]
async fn sender_stream() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<f32>>(32);

    let tcp_stream = TcpStream::connect("192.168.15.21:7777").await?;
    let mut tcp_stream = tcp_stream;

    // Spawn a task to write audio data to the TCP stream
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            let bytes = bytemuck::cast_slice(&data);

            // Get current timestamp in milliseconds
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let timestamp = now.as_millis() as u64;
            let timestamp_bytes = timestamp.to_le_bytes(); // 8 bytes

            // Prepend the size of the chunk so the receiver knows how much to read
            let size_bytes = (bytes.len() as u32).to_le_bytes(); // Use u32 for length
            if let Err(e) = tcp_stream.write_all(&size_bytes).await {
                eprintln!("Failed to send audio size: {:?}", e);
                break;
            }

            if let Err(e) = tcp_stream.write_all(&timestamp_bytes).await {
                eprintln!("Failed to send timestamp: {:?}", e);
                break;
            }

            if let Err(e) = tcp_stream.write_all(bytes).await {
                eprintln!("Failed to send audio data: {:?}", e);
                break;
            }
        }
    });

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no input available");

    let sample_rate = cpal::SampleRate(44100); // Or 48000
    let config = StreamConfig {
        channels: 2,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    println!("Using input device: {}", device.name()?);
    println!("Using sample rate: {:?}", config.sample_rate);
    println!("Using channels: {:?}", config.channels);

    let stream = device.build_input_stream(
        &config,
        {
            let tx = tx.clone();
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Corrected callback signature
                let chunk_size = data.len().min(FRAME_SIZE * config.channels as usize); // Ensure chunk fits FRAME_SIZE * channels
                let chunk = data[..chunk_size].to_vec(); // Take the appropriate chunk size
                // Send audio data to the async task
                let _ = tx.try_send(chunk);
            }
        },
        move |err| println!("Error: {:?}", err),
        None,
    )?;

    stream.play()?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
