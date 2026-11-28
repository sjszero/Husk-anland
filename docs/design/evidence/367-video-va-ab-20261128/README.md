# 367 — VA-API A/B（2026-11-28）

| 片源 | 规格 | 结果 |
|---|---|---|
| husk-video-smoke.mp4 | 1280×720 Constrained Baseline，90 帧 | PASS 90/90，约 2 s |
| hp720.mp4 | 1280×720 High，171 帧 | FAIL，2 帧后停摆 |
| bl1080.mp4 | 1920×1080 Constrained Baseline，171 帧 | FAIL，12 帧后停摆 |
| husk-normal-park.mp4 | 1920×1080 High，171 帧 | FAIL，15–25 帧后停摆 |

- 取出的 1080p High 帧 `va-b-f10.i420` 与 ffmpeg 参考帧逐像素相同（diff 0.0）。
- 停摆时 `FRAME_NONE`，`pending=0`，app `frames=4`；logcat `[in=4 pipeline=0 out=13]`。
