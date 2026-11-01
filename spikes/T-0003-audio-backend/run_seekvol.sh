#!/bin/bash
EXE=./target/release/t0003-audio-spike.exe
for be in cpal wasapi; do
  for f in wav16 flac16 mp3lame alac aac; do
    $EXE seekvol $be $f 48000
  done
done
