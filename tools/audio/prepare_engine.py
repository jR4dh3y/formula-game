#!/usr/bin/env python3
"""Build engine loops from the user's F1 2013 video reference (240–305 s).

Usage: python tools/audio/prepare_engine.py /path/to/reference-240-305.mp4
Requires ffmpeg, NumPy and SciPy. See game/audio/reference/CREDITS.md.
Preserves the recording's combustion texture instead of synthesizing harmonics.
"""
import argparse
from pathlib import Path
import subprocess
import wave

import numpy as np
from scipy.ndimage import gaussian_filter1d, median_filter
from scipy.signal import butter, sosfiltfilt

RATE = 48000
REFERENCE_RPM = 18000
BANK_FIRING = REFERENCE_RPM / 30
CYCLE = 320  # full four-stroke cycle at 18,000 RPM


def prepare(signal, start, end, frequency_range):
    clip = signal[int(start * RATE):int(end * RATE)].copy()
    # Track this engine's bank-firing order; surrounding cars and wind stay quieter.
    size, hop = 2048, 128
    centers = np.arange(size // 2, len(clip) - size // 2, hop)
    frequencies = np.fft.rfftfreq(8192, 1 / RATE)
    valid = np.flatnonzero((frequencies >= frequency_range[0]) & (frequencies <= frequency_range[1]))
    pitches = []
    for center in centers:
        spectrum = np.abs(np.fft.rfft(clip[center-size//2:center+size//2] * np.hanning(size), 8192))
        index = valid[np.argmax(spectrum[valid])]
        a, b, c = np.log(spectrum[index-1:index+2] + 1e-12)
        offset = np.clip(0.5 * (a-c) / (a-2*b+c), -0.5, 0.5)
        pitches.append((index + offset) * RATE / 8192)
    pitches = gaussian_filter1d(median_filter(pitches, size=5), sigma=2)
    frequency = np.interp(np.arange(len(clip)), centers, pitches)
    phase = np.cumsum(frequency) / RATE
    uniform_phase = np.arange(phase[0], phase[-1], BANK_FIRING / RATE)
    clip = np.interp(uniform_phase, phase, clip)
    # Remove rumble/DC and ultrasonic noise, retaining the reference's broad engine texture.
    clip = sosfiltfilt(butter(3, [100, 11000], btype="bandpass", fs=RATE, output="sos"), clip)
    envelope = np.sqrt(gaussian_filter1d(clip * clip, sigma=RATE * 0.07) + 1e-6)
    clip *= np.clip(np.median(envelope) / envelope, 0.7, 1.4)
    length = len(clip) // CYCLE * CYCLE
    clip = clip[:length]
    overlap = CYCLE * 12
    fade = 0.5 - 0.5 * np.cos(np.linspace(0, np.pi, overlap))
    # End-to-start overlap preserves real cycles and prevents a repeating boundary click.
    tail = clip[-overlap:] * (1-fade) + clip[:overlap] * fade
    result = np.concatenate([clip[overlap:-overlap], tail])
    result -= result.mean()
    result *= 0.19 / np.sqrt(np.mean(result * result))
    return result, (float(min(pitches)), float(max(pitches)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    args = parser.parse_args()
    raw = subprocess.check_output(["ffmpeg", "-v", "error", "-i", str(args.source), "-vn", "-f", "f32le", "-ar", str(RATE), "-ac", "1", "pipe:1"])
    signal = np.frombuffer(raw, "<f4").astype(float)
    on, on_pitch = prepare(signal, 0.25, 1.45, (530, 625))
    off, off_pitch = prepare(signal, 14.2, 16.5, (290, 435))
    # Align the two banks' average four-stroke cycles before load blending.
    a, b = on.reshape(-1, CYCLE).mean(0), off.reshape(-1, CYCLE).mean(0)
    shift = max(range(CYCLE), key=lambda n: np.dot(a, np.roll(b, n)))
    off = np.roll(off, shift)
    # Reserve shared headroom; never hard-clip either source.
    scale = min(1., 0.75 / max(np.max(np.abs(on)), np.max(np.abs(off))))
    output = Path(__file__).resolve().parents[2] / "game/audio/reference"
    output.mkdir(parents=True, exist_ok=True)
    for name, samples, pitch in [("engine_on.wav", on, on_pitch), ("engine_off.wav", off, off_pitch)]:
        with wave.open(str(output / name), "wb") as wav:
            wav.setparams((1, 2, RATE, 0, "NONE", "not compressed"))
            wav.writeframes(np.round(samples * scale * 32767).astype("<i2").tobytes())
        print(f"{name}: {len(samples)/RATE:.3f}s, source bank order {pitch[0]:.1f}–{pitch[1]:.1f} Hz, reference {REFERENCE_RPM} RPM")


if __name__ == "__main__":
    main()
