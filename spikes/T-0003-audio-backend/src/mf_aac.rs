//! AAC-LC in an M4A container, produced with the Windows Media Foundation encoder (test files only).
use anyhow::Result;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

pub fn write_aac_m4a(p: &Path, rate: u32, l: &[f64], r: &[f64]) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.unwrap();
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let url = HSTRING::from(p.to_str().unwrap());
        let writer = MFCreateSinkWriterFromURL(&url, None, &attrs)?;

        let out_t = MFCreateMediaType()?;
        out_t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        out_t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        out_t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        out_t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        out_t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        out_t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24000)?; // 192 kbit/s
        out_t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1)?;
        out_t.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?;
        let idx = writer.AddStream(&out_t)?;

        let in_t = MFCreateMediaType()?;
        in_t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        in_t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        in_t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        in_t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        in_t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        in_t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        in_t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 4)?;
        writer.SetInputMediaType(idx, &in_t, None)?;
        writer.BeginWriting()?;

        let (li, ri) = (crate::gen::to_i16(l), crate::gen::to_i16(r));
        let chunk = 4800usize;
        let mut pos = 0usize;
        while pos < li.len() {
            let n = chunk.min(li.len() - pos);
            let bytes = n * 4;
            let buf = MFCreateMemoryBuffer(bytes as u32)?;
            let mut ptr = std::ptr::null_mut();
            buf.Lock(&mut ptr, None, None)?;
            let dst = std::slice::from_raw_parts_mut(ptr as *mut i16, n * 2);
            for i in 0..n {
                dst[2 * i] = li[pos + i];
                dst[2 * i + 1] = ri[pos + i];
            }
            buf.Unlock()?;
            buf.SetCurrentLength(bytes as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime((pos as i64) * 10_000_000 / rate as i64)?;
            sample.SetSampleDuration((n as i64) * 10_000_000 / rate as i64)?;
            writer.WriteSample(idx, &sample)?;
            pos += n;
        }
        writer.Finalize()?;
        MFShutdown()?;
    }
    Ok(())
}
