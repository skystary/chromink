# Chromink image extraction adapter

Upstream: Hayro-interpret 0.7.0, exact registry source (commit recorded in .cargo_vcs_info.json). MIT / Apache-2.0 notices and original sources retained. The only application adapter is BitmapData / decode_bitmap at the end of src/lib.rs.

The adapter directly uses ImageXObject decoding and color-space resolution, with no page interpreter, font lookup or Device. It retains decoding arrays, ICC/CMYK/Indexed conversion, stencil and image masks, CCITT/JBIG2/JPEG2000 filters. Unused upstream APIs are removed from the application binary by LTO. Embed-fonts and embed-cmaps are disabled. Source resources retained here do not imply they are embedded in the executable.
