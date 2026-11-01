#!/bin/bash
# Gapless matrix: A|B playback through each backend, recording the callback output (what reaches the OS mixer).
EXE=./target/release/t0003-audio-spike.exe
for be in cpal wasapi; do
  for f in wav16 wav24 flac16 flac24 mp3lame mp3notag alac aac; do
    $EXE gapless $be $f 48000
  done
  $EXE gapless $be aac 48000 ext-trim
  for f in flac16 mp3lame alac; do
    $EXE gapless $be $f 44100
  done
done
