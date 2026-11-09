# P-RTS Nsight GPU Trace of repeated first-cut frames (memory: N8f/N8g headless recipe:
# Nsight-only implicit layer dir, no --platform, 200 ms window, four submits).
#   bash tools/p-rts/trace.sh NAME EXE   (inside tools/p-rts/lease.py)
NGFX="C:/Program Files/NVIDIA Corporation/Nsight Graphics 2026.4.0/host/windows-desktop-nomad-x64/ngfx.exe"
R=D:/Coding/renderlab-work/p-rts/nsight
name=$1; exe=$2
mkdir -p $R/$name/run
cp "$exe" $R/$name/trace.exe
(
  unset VK_INSTANCE_LAYERS VK_LAYER_ENABLES
  export VK_IMPLICIT_LAYER_PATH="E:/renderlab-work/n8f/layers"
  "$NGFX" --activity "GPU Trace Profiler" --output-dir "$R/$name" --exe "$R/$name/trace.exe" --dir "$R/$name" \
    --args "D:/Coding/renderlab-work/p-rts/scenes/game-base-cutprofile.json rt $R/$name/run/capture" \
    --env "RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK;" \
    --start-after-ms 7000 --limit-to-submits 4 --max-duration-ms 200 \
    --set-gpu-clocks unaltered --collect-screenshot 0 --architecture Ada --metric-set-name "Top-Level Triage"
  echo "ngfx $name exit $?"
)
taskkill //IM trace.exe //F 2>/dev/null; sleep 2
ls -la $R/$name
