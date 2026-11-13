cargo build --release --target x86_64-pc-windows-msvc
copy "target\x86_64-pc-windows-msvc\release\driver_pose_holder.dll" "C:\Program Files (x86)\Steam\steamapps\common\SteamVR\drivers\pose_holder\bin\win64\"
