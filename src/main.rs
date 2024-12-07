use std::env;
use std::error::Error;
use std::fs::read;
use rayon::prelude::*;

use image::{ImageBuffer, RgbaImage};


struct QOI {
    name: String,
    magic: [char; 4],
    width: u32,
    height: u32,
    channels: u8,
    colorspace: u8,
    data: Vec<u8>,
}

impl QOI {
    fn new(path: &str) -> Result<Self, Box<dyn Error>> {
        let name = path.to_string();
        let mut buffer = read(path).unwrap();

        if buffer.len() < 14 {
            return Err("File too small".into());
        }

        let magic = [buffer[0] as char, buffer[1] as char, buffer[2] as char, buffer[3] as char];

        if magic != ['q', 'o', 'i', 'f'] {
            return Err("Invalid magic number".into());
        }

        let width = (buffer[4] as u32) << 24 | (buffer[5] as u32) << 16 | (buffer[6] as u32) << 8 | buffer[7] as u32;
        let height = (buffer[8] as u32) << 24 | (buffer[9] as u32) << 16 | (buffer[10] as u32) << 8 | buffer[11] as u32;
        let channels = buffer[12];
        let colorspace = buffer[13];
        let data = buffer.split_off(14);

        Ok(Self{
            name: name,
            magic: magic,
            width: width,
            height: height,
            channels: channels,
            colorspace: colorspace,
            data: data,
        })
    }
}


fn decode_pixels(file: &mut QOI) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut index = [[0u8; 4]; 64];
    let mut pixels = Vec::with_capacity((file.width * file.height * 4) as usize);     
    let mut prev_pixel = [0u8, 0u8, 0u8, 255u8]; // Start with a black pixel

    let mut i = 0;
    while i < file.data.len() {
        if i + 8 <= file.data.len() && &file.data[i..i + 8] == &[0, 0, 0, 0, 0, 0, 0, 1] {
            break; // End of file marker
        }

        let pixel = file.data[i];

        match pixel {
            0b11111110 => { // QOI_OP_RGB
                // println!("QOI_OP_RGB at index {}", i);
                i += handle_rgb(&mut file.data, &mut pixels, &mut prev_pixel, i)?;
            }
            0b11111111 => { // QOI_OP_RGBA
                // println!("QOI_OP_RGBA at index {}", i);
                i += handle_rgba(&mut file.data, &mut pixels, &mut prev_pixel, i)?;
            }
            _ if (pixel >> 6) == 0b00 => { // QOI_OP_INDEX
                // println!("QOI_OP_INDEX at index {}", i);
                i += handle_index(&mut index, &mut pixels, &mut prev_pixel, pixel)?;
            }
            _ if (pixel >> 6) == 0b01 => { // QOI_OP_DIFF
                // println!("QOI_OP_DIFF at index {}", i);
                i += handle_diff(&mut pixels, &mut prev_pixel, pixel)?;
            }
            _ if (pixel >> 6) == 0b10 => { // QOI_OP_LUMA
                // println!("QOI_OP_LUMA at index {}", i);
                i += handle_luma(&mut file.data, &mut pixels, &mut prev_pixel, i)?;
            }
            _ if (pixel >> 6) == 0b11 => { // QOI_OP_RUN
                // println!("QOI_OP_RUN at index {}", i);
                i += handle_run(&mut pixels, &mut prev_pixel, pixel, file.width, file.height)?;
            }
            _ => return Err(format!("Unknown QOI operation: {:08b}", pixel).into()),
        }

        // Update the index after each pixel
        let hash = ((prev_pixel[0] as u32 * 3
            + prev_pixel[1] as u32 * 5
            + prev_pixel[2] as u32 * 7
            + prev_pixel[3] as u32 * 11)
            % 64) as usize;
        index[hash] = prev_pixel;
        // println!("Updated index at hash {}: {:?}", hash, prev_pixel);
    }

    let expected_len = (file.width as usize) * (file.height as usize) * 4; // RGBA requires 4 bytes per pixel
    if pixels.len() > expected_len {
        pixels.truncate(expected_len); // Trim extra pixels
    }
    if pixels.len() != expected_len {
        return Err(format!("Pixel data length mismatch: expected {}, got {}", expected_len, pixels.len()).into());
    }
    Ok(pixels)
}


fn handle_rgb(data: &[u8], pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], i: usize) -> Result<usize, Box<dyn std::error::Error>> {
    if i + 3 >= data.len() {
        return Err("Unexpected end of file for QOI_OP_RGB".into());
    }

    prev_pixel[0] = data[i + 1]; // R
    prev_pixel[1] = data[i + 2]; // G
    prev_pixel[2] = data[i + 3]; // B
    prev_pixel[3] = 255; // Alpha for RGB

    pixels.extend_from_slice(prev_pixel); // Add the pixel to the pixels list
    Ok(4) // The length of the RGB data is 4 bytes (1 byte for the operation + 3 bytes for RGB)
}


fn handle_rgba(data: &[u8], pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], i: usize) -> Result<usize, Box<dyn std::error::Error>> {
    if i + 4 >= data.len() {
        return Err("Unexpected end of file for QOI_OP_RGBA".into());
    }

    prev_pixel[0] = data[i + 1]; // R
    prev_pixel[1] = data[i + 2]; // G
    prev_pixel[2] = data[i + 3]; // B
    prev_pixel[3] = data[i + 4]; // A

    pixels.extend_from_slice(prev_pixel); // Add the pixel to the pixels list
    Ok(5) // The length of the RGBA data is 5 bytes (1 byte for the operation + 4 bytes for RGBA)
}

fn handle_index(index: &mut [[u8; 4]; 64], pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], pixel: u8) -> Result<usize, Box<dyn std::error::Error>> {
    let idx = pixel as usize;
    if idx >= index.len() {
        return Err("Index out of bounds for QOI_OP_INDEX".into());
    }
    *prev_pixel = index[idx];
    pixels.extend_from_slice(prev_pixel);
    Ok(1)
}

fn handle_diff(pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], pixel: u8) -> Result<usize, Box<dyn std::error::Error>> {
    let dr = ((pixel >> 4) & 0x03).wrapping_sub(2);
    let dg = ((pixel >> 2) & 0x03).wrapping_sub(2);
    let db = (pixel & 0x03).wrapping_sub(2);

    prev_pixel[0] = prev_pixel[0].wrapping_add(dr); // Apply the diff to the previous red channel
    prev_pixel[1] = prev_pixel[1].wrapping_add(dg); // Apply the diff to the previous green channel
    prev_pixel[2] = prev_pixel[2].wrapping_add(db); // Apply the diff to the previous blue channel

    pixels.extend_from_slice(prev_pixel); // Add the updated pixel to the list
    Ok(1) // Only 1 byte for QOI_OP_DIFF
}

fn handle_luma(data: &[u8], pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], i: usize) -> Result<usize, Box<dyn std::error::Error>> {
    if i + 1 >= data.len() {
        return Err("Unexpected end of file for QOI_OP_LUMA".into());
    }
    let vg = (data[i] & 0b00111111).wrapping_sub(32);
    let second_byte = data[i + 1];
    let dr_dg = ((second_byte >> 4) & 0b1111).wrapping_sub(8);
    let db_dg = (second_byte & 0b1111).wrapping_sub(8);

    prev_pixel[0] = prev_pixel[0].wrapping_add((vg as i8 + dr_dg as i8) as u8);
    prev_pixel[1] = prev_pixel[1].wrapping_add(vg as i8 as u8);
    prev_pixel[2] = prev_pixel[2].wrapping_add((vg as i8 + db_dg as i8) as u8);
    pixels.extend_from_slice(prev_pixel);
    Ok(2)
}

fn handle_run(pixels: &mut Vec<u8>, prev_pixel: &mut [u8; 4], pixel: u8, width: u32, height: u32) -> Result<usize, Box<dyn std::error::Error>> {
    let run_length = (pixel & 0x3F) + 1;
    for _ in 0..run_length {
        pixels.extend_from_slice(prev_pixel);
    }
    Ok(1) // We need to increment `i` by 1 since `QOI_OP_RUN` only takes 1 byte
}
 
fn save_as_image(file: &QOI, pixels: &[u8]) -> Result<(), Box<dyn Error>> {
    let width = file.width;
    let height = file.height;

    let img: RgbaImage = ImageBuffer::from_raw(width, height, pixels.to_vec())
        .ok_or("Failed to create image buffer")?;

    img.save("output.png")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        println!("Usage: cargo run /path/to/image.qoi");
        return Ok(());
    }

    let path = &args[1];
    let mut file = QOI::new(path)?;

    println!(
        "name: {}, magic: {:?}, width: {}, height: {}, channels: {}, colorspace: {}, data: {}",
        file.name, file.magic, file.width, file.height, file.channels, file.colorspace, file.data.len()
    );
    let pixels = decode_pixels(&mut file)?;
    save_as_image(&file, &pixels)?;

    save_as_image(&file, &pixels)?;
    println!("Saved image as output.png");
    Ok(())
}


