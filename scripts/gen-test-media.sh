#!/usr/bin/env bash
# Generate small synthetic test media (all with real audio unless stated) for
# integration tests. Everything is produced locally from FFmpeg's lavfi sources;
# no personal videos are read. Usage: scripts/gen-test-media.sh [out_dir] [ffmpeg_dir]
set -euo pipefail

OUT="${1:-$(cd "$(dirname "$0")/.." && pwd)/test-media}"
BIN="${2:-}"
mkdir -p "$OUT"
# Make both paths absolute before changing directory: callers such as CI pass relative ones.
OUT="$(cd "$OUT" && pwd)"
if [ -n "$BIN" ]; then BIN="$(cd "$BIN" && pwd)"; fi
FF="${BIN:+$BIN/}ffmpeg"
cd "$OUT"

ff() { "$FF" -hide_banner -loglevel error -y "$@"; }
V="testsrc2=size=640x360:rate=30,format=yuv420p"
A="sine=frequency=440:sample_rate=48000"
A2="sine=frequency=660:sample_rate=48000"

echo "→ writing test media to $OUT"

# 1. H.264 + AAC stereo, 4 s, MP4
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 4 -c:v libx264 -preset veryfast -crf 23 -g 60 \
   -c:a aac -b:a 128k -ac 2 -movflags +faststart h264_aac.mp4

# 2. AV1 (SVT-AV1) + Opus, MKV
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 4 -c:v libsvtav1 -preset 10 -crf 35 -g 60 \
   -c:a libopus -b:a 96k av1_opus.mkv

# 3. HEVC 10-bit + AAC, MKV
ff -f lavfi -i "testsrc2=size=640x360:rate=25,format=yuv420p10le" -f lavfi -i "$A" -t 3 \
   -c:v libx265 -preset ultrafast -crf 26 -x265-params log-level=error -c:a aac -b:a 128k hevc_10bit.mkv

# 4. No audio track
ff -f lavfi -i "$V" -t 3 -c:v libx264 -preset veryfast -crf 23 no_audio.mp4

# 5. Multiple audio tracks (AAC stereo zho + AC-3 5.1 eng) + SRT subtitle, MKV
printf '1\n00:00:00,500 --> 00:00:02,000\n你好，世界\n\n2\n00:00:02,200 --> 00:00:03,500\nこんにちは\n' > subs_zh.srt
ff -f lavfi -i "$V" -f lavfi -i "$A" -f lavfi -i "$A2" -i subs_zh.srt -t 4 \
   -map 0:v -map 1:a -map 2:a -map 3:s -c:v libx264 -preset veryfast -crf 23 \
   -c:a:0 aac -b:a:0 128k -ac:a:0 2 -c:a:1 ac3 -b:a:1 384k -ac:a:1 6 -c:s srt \
   -metadata:s:a:0 language=zho -metadata:s:a:0 title="中文 立体声" \
   -metadata:s:a:1 language=eng -metadata:s:a:1 title="English 5.1" \
   -metadata:s:s:0 language=zho multi_audio.mkv

# 6. PCM audio in MOV (cannot be stream-copied into MP4)
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 3 -c:v libx264 -preset veryfast -crf 23 -c:a pcm_s16le pcm_audio.mov

# 7. Variable frame rate: 45 frames at 30 fps, then 15 fps
ff -f lavfi -i "testsrc2=size=640x360:rate=30" -f lavfi -i "$A" -t 4 \
   -vf "setpts='if(lt(N,45),N/30/TB,(45/30+(N-45)/15)/TB)',format=yuv420p" -fps_mode vfr \
   -c:v libx264 -preset veryfast -crf 23 -c:a aac -b:a 128k -shortest vfr.mkv

# 8. Rotated phone clip: display matrix 90° (stored landscape, shown portrait)
ff -display_rotation:v:0 -90 -i h264_aac.mp4 -c copy rotated.mp4

# 9. HDR10 (PQ, BT.2020, mastering display + MaxCLL), HEVC 10-bit
# FFmpeg 8 takes colour properties from frames, so tag them with setparams.
ff -f lavfi -i "testsrc2=size=640x360:rate=24,format=yuv420p10le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv" -f lavfi -i "$A" -t 3 \
   -c:v libx265 -preset ultrafast -crf 24 \
   -x265-params "log-level=error:hdr10=1:repeat-headers=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400" \
   -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -color_range tv \
   -c:a aac -b:a 128k hdr10.mkv

# 10. 4:4:4 H.264 (SVT-AV1 cannot keep 4:4:4)
ff -f lavfi -i "testsrc2=size=640x360:rate=30,format=yuv444p" -f lavfi -i "$A" -t 3 \
   -c:v libx264 -preset veryfast -crf 20 -c:a aac -b:a 128k yuv444.mkv

# 11. Full-range (JPEG range) H.264
ff -f lavfi -i "testsrc2=size=640x360:rate=30,format=yuvj420p" -f lavfi -i "$A" -t 3 \
   -c:v libx264 -preset veryfast -crf 23 -color_range pc -c:a aac -b:a 128k fullrange.mp4

# 12. Longer clip for cancel / progress tests (20 s, 720p)
ff -f lavfi -i "testsrc2=size=1280x720:rate=30,format=yuv420p" -f lavfi -i "$A" -t 20 \
   -c:v libx264 -preset veryfast -crf 23 -c:a aac -b:a 128k long_720p.mp4

# 13. Corrupt inputs: truncated MP4 (no moov) and random bytes with a video extension
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 3 -c:v libx264 -preset veryfast -c:a aac nofaststart.mp4
head -c 60000 nofaststart.mp4 > corrupt_truncated.mp4
rm -f nofaststart.mp4
head -c 200000 /dev/urandom > corrupt_random.mkv

# 13b. Index intact, data cut off (an unfinished download): moov at the front declares
# 10 s, but only the first ~55% of the file is present.
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 10 -c:v libx264 -preset veryfast -c:a aac -movflags +faststart whole.mp4
head -c $(( $(wc -c < whole.mp4) * 55 / 100 )) whole.mp4 > incomplete_download.mp4
rm -f whole.mp4

# 13c. MOV edit list hiding leading frames (as phone editors write when trimming without
# re-encoding): a stream copy from a non-keyframe start keeps the GOP's earlier frames as
# discarded packets, so the file holds more video packets than it shows.
ff -f lavfi -i "$V" -f lavfi -i "$A" -t 4 -c:v libx264 -preset veryfast -g 30 -c:a aac gop30.mp4
ff -ss 0.5 -i gop30.mp4 -c copy edit_list.mov
rm -f gop30.mp4

# 14. Unicode / special-character paths
mkdir -p "中文 目录/日本語 フォルダ"
cp h264_aac.mp4 "中文 目录/旅行 vlog #1 (最终版) & 東京タワー 'test'.mp4"
cp av1_opus.mkv "中文 目录/日本語 フォルダ/猫の動画 [AV1] 100%.mkv"

echo "✓ done"; ls -la "$OUT"
