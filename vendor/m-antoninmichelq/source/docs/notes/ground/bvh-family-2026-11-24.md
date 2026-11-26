# Root-UAV computes = DXR acceleration-structure builders (offline, 2026-11-24 ~21:45 Paris)
Dumps staged from jouer lumiere run (local only). Translated offline on 0c313ea with ROOT_UAVS=1.
- caa78b8c / 28918b71: box16/box32 BVH node writers (crash with copy-back) -> excluded by 0c313ea.
- 5062bc7a (21.6 KB, global loads + global_atomic_fmin/fmax), 5279ae36 (dwordx3 AABB stores, atomic add), c42a3195 (copy + +-FLT_MAX AABB init), 3cea4534 (instance descriptors via global stores, +-FLT_MAX): BVH front-end (bounds / instances). All three of the "still dropped" now TRANSLATE OK with root UAVs; 3cea4534 still refuses (chased write).
- The 10 "safe" ones (4ddd6f97 3b90165a 5552dac6 162216b9 3df629ff 6f6a359d f28a33a8 4b5aedc0 f6e47efa 5e687674) are the same family (sorts, AABB fmin/fmax, node builds), as ground-2026-11-24.md already suspected.
=> None of these produce the mesh dispatch arguments. Buildings/terrain (kind-10 mesh ExecuteIndirect, ~8 groups/dispatch) are fed elsewhere. Adding the 3 to ROOT_UAVS_ONLY is not expected to show buildings.

## Mesh path (from patterns/mesh-indirect-lumiere.txt, 10-calculs diag)
- ~3650 kind-10 mesh dispatches/frame, 0 all-zero, 0.02-0.03M groups (5-8 each); 0 mesh shaders untranslated (280/339 OK).
- Only kind-10 drops: pixel 93667197 / b15ba489 = PS_DeferredPedTexturedNC_Packed (characters, "packed input half"; Characters thread). Not buildings.
- So building meshes are not blocked by translation. Remaining hypotheses: (1) the per-dispatch group counts are too small (counts from a CPU/GPU path that gives ~1 meshlet batch per object, e.g. LOD/visibility lists never filled), (2) buildings are drawn but at wrong positions/culled by depth (instance transforms from a buffer not refreshed), (3) building draws use another path (non-indirect) dropped elsewhere. Needs one frame capture naming mesh shaders per dispatch (bundled into a Jouer run only).
