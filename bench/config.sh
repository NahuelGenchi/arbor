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
# seconds, under the load of FRAMES_CELLS and FRAMES_SPIKE_MIB. Empty FLUTTER: flutter on PATH; empty FLUTTER_PUB_CACHE:
# PUB_CACHE, else ~/.pub-cache, where the packages Flutter pins already are (the build is offline).
TOOLKITS="${TOOLKITS:-gtk4 gtk4rs qtquick slint iced flutter}"
TOOLKITS_PREBUILT="${TOOLKITS_PREBUILT:-}"
TOOLKITS_LAUNCHES="${TOOLKITS_LAUNCHES:-5}"
TOOLKITS_STILL_S="${TOOLKITS_STILL_S:-60}"
TOOLKITS_SETTLE_S="${TOOLKITS_SETTLE_S:-90}"
FLUTTER="${FLUTTER:-}"
FLUTTER_PUB_CACHE="${FLUTTER_PUB_CACHE:-}"

# 11. Mission Control (prototype/mission), measured only with ONLY=mission, as the toolkits are: empty MISSION_BIN builds
# it with arbor (release, offline) into the temp dir, a path measures that binary instead, with ARBOR_BIN; the rooms of
# its busy state, each a sleeping cell under a memory limit.
MISSION_BIN="${MISSION_BIN:-}"
MISSION_ROOMS="${MISSION_ROOMS:-300}"

# 12. The five goals, measured only with ONLY=goals: the commit of this repository the work runs on; the parts (empty:
# session power safety ease still); the measured runs of each arm, after one warm-up; the still desktop's blocks and
# their seconds, per arm. Its temp folder is made on disk in GOALS_TMP (empty: RESULTS_DIR), so builds write a real
# disk, and is removed at exit like the other.
GOALS_REF="${GOALS_REF:-1e314a0ae6a8669098c8991cbcc94837e349f6e6}"
GOALS_PARTS="${GOALS_PARTS:-}"
GOALS_RUNS="${GOALS_RUNS:-3}"
GOALS_STILL_S="${GOALS_STILL_S:-150}"
GOALS_STILL_BLOCKS="${GOALS_STILL_BLOCKS:-2}"
GOALS_TMP="${GOALS_TMP:-}"

# Where temp files go (removed at exit; tmpfs, as in the M run) and where reports are written.
BENCH_TMP="${BENCH_TMP:-${TMPDIR:-/tmp}}"
RESULTS_DIR="${RESULTS_DIR:-$BENCH_DIR/results}"
