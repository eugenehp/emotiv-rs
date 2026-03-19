//! AES-ECB packet decryption and data extraction.
//!
//! Implements the full decryption pipeline matching CortexService binary:
//! - AES key derivation from serial number
//! - AES-128-ECB decryption
//! - Bit unpacking of EEG samples
//! - Contact quality extraction
//! - ADC to microvolts conversion

use crate::raw::types::{DecryptedData, HeadsetModel};
use anyhow::{anyhow, Result};

/// Full decrypted packet state.
pub struct Decryptor {
    model: HeadsetModel,
    #[allow(dead_code)]
    serial: String,
    aes_key: Vec<u8>,
    counter: u32,
    physical_min: f64,
    physical_max: f64,
    digital_max: i32,
}

impl Decryptor {
    /// Create a new decryptor for a headset.
    pub fn new(model: HeadsetModel, serial: String) -> Result<Self> {
        let aes_key = derive_aes_key_for_model(model, &serial)?;

        // Physical ranges per model
        let (physical_min, physical_max) = model.eeg_physical_range();
        let digital_max = match model {
            HeadsetModel::EpocX | HeadsetModel::EpocPlus | HeadsetModel::EpocStd | HeadsetModel::EpocFlex => {
                16383i32
            }
            HeadsetModel::Insight | HeadsetModel::Insight2 => 16383i32,
            HeadsetModel::MN8 => 16383i32,
            HeadsetModel::Xtrodes => 65535i32,
        };

        Ok(Self {
            model,
            serial,
            aes_key,
            counter: 0,
            physical_min,
            physical_max,
            digital_max,
        })
    }

    /// Decrypt and extract EEG data from a BLE/USB packet.
    pub fn decrypt_eeg_packet(&mut self, encrypted_data: &[u8]) -> Result<DecryptedData> {
        if encrypted_data.len() < 20 {
            return Err(anyhow!("Packet too short: {} bytes", encrypted_data.len()));
        }

        // Decrypt using AES-ECB
        let decrypted = aes_ecb_decrypt(&self.aes_key, encrypted_data)?;

        // Extract counter (bytes 0-1)
        let packet_counter = u16::from_be_bytes([decrypted[0], decrypted[1]]) as u32;
        self.counter = packet_counter;

        // Extract EEG data (model-dependent sample packing)
        let eeg_channels = self.model.channel_count();
        let eeg_adc = extract_eeg_samples(self.model, &decrypted, eeg_channels);
        let min_channels = min_required_channels(self.model);
        if eeg_adc.len() < min_channels {
            return Err(anyhow!(
                "Incomplete EEG payload for {}: got {} of {} channels",
                self.model.name(),
                eeg_adc.len(),
                eeg_channels
            ));
        }
        let eeg_uv = self.adc_to_uv(&eeg_adc);

        // Extract contact quality (CQ in high bits)
        let contact_quality = extract_contact_quality(&decrypted, eeg_channels);
        let signal_quality = calculate_signal_quality(&contact_quality);

        // Extract battery (last byte typically)
        let battery_percent = extract_battery(&decrypted);

        let data = DecryptedData::new(
            packet_counter,
            eeg_uv,
            eeg_adc.iter().map(|&x| x as i32).collect(),
            contact_quality,
            signal_quality,
            battery_percent,
        );

        Ok(data)
    }

    /// Convert ADC values to microvolts.
    fn adc_to_uv(&self, adc: &[u16]) -> Vec<f64> {
        adc.iter()
            .map(|&val| {
                let normalized = (val as f64) / (self.digital_max as f64);
                self.physical_min + (normalized * (self.physical_max - self.physical_min))
            })
            .collect()
    }
}

fn min_required_channels(model: HeadsetModel) -> usize {
    match model {
        HeadsetModel::Insight | HeadsetModel::Insight2 => 5,
        HeadsetModel::MN8 => 2,
        HeadsetModel::Xtrodes => 8,
        HeadsetModel::EpocX
        | HeadsetModel::EpocPlus
        | HeadsetModel::EpocStd
        | HeadsetModel::EpocFlex => 10,
    }
}

fn derive_aes_key_for_model(model: HeadsetModel, serial: &str) -> Result<Vec<u8>> {
    match model {
        HeadsetModel::EpocX => derive_aes_key_epoc_x_mode(serial),
        HeadsetModel::EpocPlus | HeadsetModel::EpocFlex | HeadsetModel::Insight2 => {
            derive_aes_key_v2(serial)
        }
        HeadsetModel::EpocStd | HeadsetModel::Insight | HeadsetModel::MN8 | HeadsetModel::Xtrodes => {
            derive_aes_key_v1(serial)
        }
    }
}

/// Derive AES key from serial number (v1 algorithm, matches binary).
fn derive_aes_key_v1(serial: &str) -> Result<Vec<u8>> {
    if serial.len() < 12 {
        return Err(anyhow!(
            "Serial number must be at least 12 chars, got: {}",
            serial.len()
        ));
    }

    let bytes = serial.as_bytes();
    let mut key = vec![0u8; 16];

    // Key derivation indices matching CortexService binary
    key[0] = bytes[1];
    key[1] = 0x00;
    key[2] = bytes[2];
    key[3] = bytes[3];
    key[4] = bytes[4];
    key[5] = bytes[8];
    key[6] = bytes[9];
    key[7] = bytes[7];
    key[8] = bytes[10];
    key[9] = bytes[11];
    key[10] = bytes[0];
    key[11] = bytes[6];
    key[12] = bytes[5];
    key[13] = 0x54; // 'T'
    key[14] = 0x10;
    key[15] = 0x42; // 'B'

    Ok(key)
}

/// Derive AES key from serial number (v2 algorithm, matches benchmark parity).
fn derive_aes_key_v2(serial: &str) -> Result<Vec<u8>> {
    if serial.len() < 12 {
        return Err(anyhow!(
            "Serial number must be at least 12 chars, got: {}",
            serial.len()
        ));
    }

    let bytes = serial.as_bytes();
    let mut key = vec![0u8; 16];
    key[0] = bytes[0];
    key[1] = bytes[3];
    key[2] = bytes[7];
    key[3] = bytes[1];
    key[4] = bytes[5];
    key[5] = bytes[11];
    key[6] = bytes[4];
    key[7] = bytes[9];
    key[8] = bytes[2];
    key[9] = bytes[6];
    key[10] = bytes[10];
    key[11] = bytes[8];
    key[12] = 0x31; // '1'
    key[13] = 0x31; // '1'
    key[14] = 0x35; // '5'
    key[15] = 0x39; // '9'
    Ok(key)
}

/// Derive AES key from serial number (EPOC X mode variant).
fn derive_aes_key_epoc_x_mode(serial: &str) -> Result<Vec<u8>> {
    if serial.len() < 12 {
        return Err(anyhow!(
            "Serial number must be at least 12 chars, got: {}",
            serial.len()
        ));
    }

    let bytes = serial.as_bytes();
    let mut key = vec![0u8; 16];
    key[0] = bytes[0];
    key[1] = bytes[1];
    key[2] = bytes[2];
    key[3] = bytes[3];
    key[4] = bytes[4];
    key[5] = bytes[5];
    key[6] = bytes[6];
    key[7] = bytes[7];
    key[8] = bytes[8];
    key[9] = bytes[9];
    key[10] = bytes[10];
    key[11] = bytes[11];
    key[12] = 0x45; // 'E'
    key[13] = 0x58; // 'X'
    key[14] = 0x31; // '1'
    key[15] = 0x30; // '0'
    Ok(key)
}

/// AES-ECB decryption.
#[cfg(feature = "raw")]
fn aes_ecb_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    use aes::Aes128;
    use cipher::{KeyInit, BlockDecrypt};

    let cipher = Aes128::new_from_slice(key)
        .map_err(|_| anyhow!("Invalid AES key length"))?;

    let mut plaintext = Vec::with_capacity(ciphertext.len());
    
    for chunk in ciphertext.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        let block_arr: aes::Block = block.into();
        let mut decrypted = block_arr;
        cipher.decrypt_block(&mut decrypted);
        plaintext.extend_from_slice(&decrypted);
    }
    
    Ok(plaintext[..ciphertext.len()].to_vec())
}

/// Extract 14-bit EEG samples from packed bytes.
fn extract_14bit_samples(data: &[u8], channel_count: usize) -> Vec<u16> {
    let mut samples = Vec::with_capacity(channel_count);
    let total_bits = data.len() * 8;
    for sample_idx in 0..channel_count {
        let start_bit = sample_idx * 14;
        if start_bit + 14 > total_bits {
            break;
        }

        let mut value: u16 = 0;
        for bit in 0..14 {
            let bit_pos = start_bit + bit;
            let byte_idx = bit_pos / 8;
            let bit_idx_in_byte = 7 - (bit_pos % 8);
            let bit_val = (data[byte_idx] >> bit_idx_in_byte) & 0x01;
            value = (value << 1) | (bit_val as u16);
        }

        samples.push(value & 0x3FFF);
    }

    samples
}

fn extract_16bit_be_samples(data: &[u8], channel_count: usize) -> Vec<u16> {
    let mut samples = Vec::with_capacity(channel_count);
    for idx in 0..channel_count {
        let offset = idx * 2;
        if offset + 1 >= data.len() {
            break;
        }
        let word = u16::from_be_bytes([data[offset], data[offset + 1]]);
        samples.push(word & 0x3FFF);
    }
    samples
}

fn extract_24bit_be_samples(data: &[u8], channel_count: usize) -> Vec<u16> {
    let mut samples = Vec::with_capacity(channel_count);
    for idx in 0..channel_count {
        let offset = idx * 3;
        if offset + 2 >= data.len() {
            break;
        }
        let value = ((data[offset] as u32) << 16)
            | ((data[offset + 1] as u32) << 8)
            | (data[offset + 2] as u32);
        samples.push(((value >> 8) & 0xFFFF) as u16);
    }
    samples
}

fn extract_eeg_samples(model: HeadsetModel, decrypted: &[u8], channel_count: usize) -> Vec<u16> {
    if decrypted.len() <= 3 {
        return Vec::new();
    }

    // Counter occupies first 2 bytes, battery is typically final byte.
    let eeg_payload = &decrypted[2..decrypted.len() - 1];

    match model {
        HeadsetModel::EpocPlus | HeadsetModel::EpocX => {
            extract_16bit_be_samples(eeg_payload, channel_count)
        }
        HeadsetModel::EpocFlex => extract_24bit_be_samples(eeg_payload, channel_count),
        _ => extract_14bit_samples(eeg_payload, channel_count),
    }
}

/// Extract per-channel contact quality.
fn extract_contact_quality(data: &[u8], channel_count: usize) -> Vec<u8> {
    let mut cq = vec![0u8; channel_count];

    // CQ is typically in the high 2 bits of specific bytes
    // This is model-dependent; for now use a simple extraction
    if data.len() >= 32 {
        for i in 0..channel_count.min(14) {
            let byte_idx = 20 + (i / 4);
            if byte_idx < data.len() {
                let nibble = (data[byte_idx] >> ((3 - (i % 4)) * 2)) & 0x03;
                cq[i] = (nibble << 1) as u8; // Map 0-3 to 0-4 range
            }
        }
    }

    cq
}

/// Calculate overall signal quality from per-channel CQ.
fn calculate_signal_quality(contact_quality: &[u8]) -> u8 {
    if contact_quality.is_empty() {
        return 0;
    }
    let avg = contact_quality.iter().map(|&x| x as u32).sum::<u32>() / contact_quality.len() as u32;
    (avg as u8).min(4)
}

/// Extract battery level from packet.
fn extract_battery(data: &[u8]) -> u8 {
    if data.is_empty() {
        return 0;
    }
    // Benchmark parity: convert raw byte to voltage then map ~3.2V..4.2V to 0..100.
    let raw_battery = data[data.len() - 1];
    let voltage = 3.0 + (raw_battery as f64 / 255.0) * 1.2;
    if voltage <= 3.2 {
        0
    } else if voltage >= 4.2 {
        100
    } else {
        (((voltage - 3.2) / (4.2 - 3.2)) * 100.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_derivation() -> Result<()> {
        let serial = "MOCK-SN-000001";
        let key = derive_aes_key_v1(serial)?;
        assert_eq!(key.len(), 16);
        assert_eq!(key[0], b'-');
        assert_eq!(key[13], 0x54); // 'T'
        Ok(())
    }

    #[test]
    fn test_key_derivation_v2() -> Result<()> {
        let serial = "MOCKSN000001";
        let key = derive_aes_key_v2(serial)?;
        assert_eq!(key.len(), 16);
        assert_eq!(key[12], b'1');
        assert_eq!(key[15], b'9');
        Ok(())
    }

    #[test]
    fn test_extract_samples() {
        // Test 14-bit unpacking
        let data = [0xFF, 0xFF, 0xFF, 0xFF]; // All ones
        let samples = extract_14bit_samples(&data, 2);
        assert_eq!(samples.len(), 2);
        assert!(samples[0] > 0);
    }
}
