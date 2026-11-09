#!/bin/bash
# P-NRT candidate builds outside the GPU lease, two jobs, in a target outside any
# `renderlab`/`renderlab-work` path segment so build scripts never trip other units' guards.
# Each build is killed, and the script exits 2, when a schedule line written after it starts
# restricts compilers (QUIET, compiler, cargo, rustc): root's compiler-quiet windows apply to
# background builds too.
#   tools/p-nrt/build.sh TARGET NAME=TREE[:lib] ...   (snapshots into p-nrt/bin/NAME.exe)
S=/e/renderlab-work/gpu-schedule.md
T=$1; shift
BIN=/d/Coding/renderlab-work/p-nrt/bin
mkdir -p "$BIN"
watch_from=$(wc -l < "$S")
run() {  # run cargo in the background; abort on a new compiler-restricting line
  "$@" &
  pid=$!
  while kill -0 $pid 2>/dev/null; do
    if tail -n +"$((watch_from + 1))" "$S" 2>/dev/null | grep -v '^\[m7-p-nrt' | grep -q -i -E 'QUIET|compiler|no cargo|rustc'; then
      taskkill //PID "$(cat /proc/$pid/winpid)" //T //F >/dev/null 2>&1
      echo "ABORTED: a new schedule line restricts compilers"; exit 2
    fi
    sleep 2
  done
  wait $pid
}
for item in "$@"; do
  name=${item%%=*}; tree=${item#*=}; kind=
  if [[ $tree == *:lib ]]; then tree=${tree%:lib}; kind=lib; fi
  if [ "$kind" = "lib" ]; then
    run env CARGO_TARGET_DIR=$T cargo test -p renderlab --lib --no-run --release -j 2 --manifest-path "$tree/Cargo.toml" \
      --message-format=json > "$BIN/$name.jsonl" 2> "$BIN/$name.err" || { echo "FAIL $name"; tail -20 "$BIN/$name.err"; exit 1; }
    exe=$(py -3 -B -c "
import json,sys
a=[json.loads(l) for l in open(sys.argv[1]) if l.startswith('{')]
print([x['executable'] for x in a if x.get('reason')=='compiler-artifact' and x.get('executable') and x['target']['name']=='renderlab'][-1])" "$BIN/$name.jsonl")
  else
    run env CARGO_TARGET_DIR=$T cargo build --release --example ara1_capture -j 2 --manifest-path "$tree/Cargo.toml" \
      > "$BIN/$name.log" 2>&1 || { echo "FAIL $name"; tail -20 "$BIN/$name.log"; exit 1; }
    exe=$T/release/examples/ara1_capture.exe
  fi
  cp "$exe" "$BIN/$name.tmp" && mv "$BIN/$name.tmp" "$BIN/$name.exe" && echo "BUILT $name $(sha256sum "$BIN/$name.exe" | cut -c1-16)"
done
echo BUILD-DONE
