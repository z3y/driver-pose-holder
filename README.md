# SteamVR Pose Holder Driver

A small SteamVR driver that freezes your controllers at their last valid pose when tracking is lost, instead of letting SteamVR treat them as idle.

## Why this exists

Quest currently has a bug where the controllers go idle too fast. They report lost tracking and fall asleep much sooner than they should, SteamVR marks them as sleeping, and games respond by moving your  hands down or away. On top of that the next pose reported to be valid is still invalid, which results in hands snapping weirdly back in place once they gain back tracking.

The goal of this driver is to work around that bug. When a controller reports lost tracking, the driver keeps feeding SteamVR the last good pose, so your hands stay where they were until tracking comes back.

## Installing

- Unzip the folder from releases into `C:\Program Files (x86)\Steam\steamapps\common\SteamVR\drivers\`.
- Make sure the folder has this layout:
```
drivers/
  pose_holder/
    driver.vrdrivermanifest
    bin/
      win64/
        driver_pose_holder.dll
```
- Restart SteamVR
