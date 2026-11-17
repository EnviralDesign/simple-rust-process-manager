# eframe 0.33.3

`eframe/` contains the crates.io sources for eframe 0.33.3 (MIT OR Apache-2.0).
The upstream licenses are alongside this file.

Our changes are in `src/epi.rs` and `src/native/wgpu_integration.rs`: an opt-in
`App::release_gpu_resources_on_minimize` hook releases the Windows wgpu painter
while continuing UI updates and normal close handling. The painter and font
atlas are restored when the window returns. This app uses only the font atlas,
one viewport, and no retained GPU handles. Other backends keep upstream behavior.

The local Cargo patch is needed because eframe owns the painter privately.
Keep these changes when updating eframe; the rest of the vendored source is upstream.
