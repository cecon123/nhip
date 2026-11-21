GPU background port from @kawarp/core 1.3.1 (MIT), npm shasum b9922d2643e5664374fbd59b8141c7be76561512.
warp.frag keeps its simplex/domain warp; output.frag keeps vignette, saturation and temporal dithering with Spicy Lyrics CSS grading and egui_glow gamma-space composition.
Reference Spicy Lyrics commit 34ad8c683204c522f227060475bf4f8aae36508e, src/css/DynamicBG/spicy-dynamic-bg.css.
CPU cover blur is precomputed once per artwork; the moving warp and final grading run on GPU.
