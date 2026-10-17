# Parameters of the baseline suite. run.sh sources this file; every value can be overridden from the
# environment (e.g. `NAME_QUERY=report ./run.sh`). Empty values are resolved by the sections as described.

# The folder that holds this project (bench/ sits in <projects>/<project>/bench).
PROJECTS_DIR="${PROJECTS_DIR:-$(cd "$BENCH_DIR/../.." && pwd)}"

# 4. Search
NAME_SEARCH_ROOT="${NAME_SEARCH_ROOT:-$HOME}"
NAME_QUERY="${NAME_QUERY:-invoice}"
CODE_ROOT="${CODE_ROOT:-$PROJECTS_DIR}"
CODE_QUERIES="${CODE_QUERIES:-parseInt|retroachievements|TODO:|fn main|border-radius}"   # '|'-separated literals

# 6. Storage
DEDUP_ROOT="${DEDUP_ROOT:-$PROJECTS_DIR}"
FORK_TREE="${FORK_TREE:-}"            # empty: the largest directory directly under PROJECTS_DIR

# 7. Tokens
TRANSCRIPTS_DIR="${TRANSCRIPTS_DIR:-$HOME/.claude/projects}"
O200K_FILE="${O200K_FILE:-/usr/share/code/resources/app/extensions/copilot/dist/o200k_base.tiktoken}"
LISTING_DIR="${LISTING_DIR:-/usr/bin}"
FORMATS_LOCALE="${FORMATS_LOCALE:-en_US.UTF-8}"
# `git log -20` starts from this commit (the one the M run saw) so the count does not drift as the repo grows.
# Empty GIT_LOG_REPO: the repository under PROJECTS_DIR that contains GIT_LOG_REF, else the most recently
# committed one (at HEAD).
GIT_LOG_REPO="${GIT_LOG_REPO:-}"
GIT_LOG_REF="${GIT_LOG_REF:-f44fb79aaf8f4e8530fa6345638bd39dd8046501}"

# 3. Memory: the heap test parses one transcript. Empty HEAP_JSONL: the transcript of exactly HEAP_JSONL_BYTES
# bytes (the M input), else the 3rd-largest one directly under TRANSCRIPTS_DIR/*/ (the M rule).
HEAP_JSONL="${HEAP_JSONL:-}"
HEAP_JSONL_BYTES="${HEAP_JSONL_BYTES:-127879200}"

# 2. Cells: an image that is already present locally; it is never pulled.
DOCKER_IMAGE="${DOCKER_IMAGE:-ubuntu:24.04}"

# 5. Boot
INITRD="${INITRD:-/boot/initrd.img}"

# 8. The prototype: empty builds prototype/ (release, offline) into the temp dir; a path measures that binary instead.
ARBOR_BIN="${ARBOR_BIN:-}"
# `arbor look outline` reads the OUTLINE_FILES largest code files of this repository at this commit, read with git, so
# the input stays the same as the code changes.
OUTLINE_REF="${OUTLINE_REF:-6bbbb22ee1321c5d5c18d0eb04d78aaa3cfef779}"
OUTLINE_FILES="${OUTLINE_FILES:-10}"

# 9. The screen: seconds of still screen (the brief's target is stated over 60 s), seconds of frames in each run, and
# the load of the second run. Empty FRAMES_CELLS: one busy cell per CPU thread. Empty FRAMES_SPIKE_MIB: the spike takes
# all the memory available when it starts but 1 GiB.
SCREEN_STILL_S="${SCREEN_STILL_S:-60}"
FRAMES_S="${FRAMES_S:-20}"
FRAMES_CELLS="${FRAMES_CELLS:-}"
FRAMES_SPIKE_MIB="${FRAMES_SPIKE_MIB:-}"

# 10. The toolkits of issue #8, measured only with ONLY=toolkits: which ones, in this order; a folder that
# toolkits/build.sh built them in before (empty: build them into the temp dir); the launches timed for the first frame;
# the seconds of still screen; the seconds the machine is left to settle after the builds and after each toolkit's
# loaded run, before the section waits for a 1-minute load average under 1. Their animation runs last FRAMES_S
# seconds, under the load of FRAMES_CELLS and FRAMES_SPIKE_MIB.
TOOLKITS="${TOOLKITS:-gtk4 qtquick slint iced}"
TOOLKITS_PREBUILT="${TOOLKITS_PREBUILT:-}"
TOOLKITS_LAUNCHES="${TOOLKITS_LAUNCHES:-5}"
TOOLKITS_STILL_S="${TOOLKITS_STILL_S:-60}"
TOOLKITS_SETTLE_S="${TOOLKITS_SETTLE_S:-90}"

# Where temp files go (removed at exit; tmpfs, as in the M run) and where reports are written.
BENCH_TMP="${BENCH_TMP:-${TMPDIR:-/tmp}}"
RESULTS_DIR="${RESULTS_DIR:-$BENCH_DIR/results}"
