# Patches

Runtimes are built as their authors publish them, with one exception the user asked for:

- **Shader cache key** (`src/fetch.rs` `shader_key_patch`): a fork whose shader cache is keyed on its whole
  `victory.exe` (today klbf-lab) throws every translated shader away on each rebuild. After fetching, i95
  rewrites that one line so the key is a hash of the fork's own translator sources (`crates/victory-shader`),
  passed as `NEONBRIDGE_SHADER_KEY`. A translator change still retranslates; a rebuild alone does not.
  `NEONBRIDGE_SHADER_KEY_PATCH=0` turns it off (on a fresh source copy).

Every other fork already keys on its translator (VIctory's `VICTORY_SHADER_CACHE_KEY` defaults to it).
