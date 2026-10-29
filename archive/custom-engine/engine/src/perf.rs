#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfError {
    SizeOverflow,
    SizeMismatch { expected: usize, actual: usize },
    OutputRejected,
}

unsafe extern "C" {
    fn gospol_hiz_reduce_rgba16(
        depth: *const f32,
        width: u32,
        height: u32,
        out_rgba16: *mut u16,
        out_len: usize,
    ) -> usize;

    fn gospol_edge_stencil_4x4(
        depth: *const f32,
        width: u32,
        height: u32,
        threshold: f32,
        out_mask: *mut u8,
        out_len: usize,
    ) -> usize;

    fn gospol_blue_noise_hash(x: u32, y: u32, frame: u32, seed: u32) -> u32;

    fn gospol_gt_tonemap_rgba8(
        hdr_rgba: *const f32,
        pixel_count: u32,
        exposure: f32,
        out_rgba8: *mut u8,
        out_len: usize,
    ) -> usize;

    fn gospol_kawase_blur_rgba32f(
        src_rgba: *const f32,
        width: u32,
        height: u32,
        radius: u32,
        dst_rgba: *mut f32,
        out_len: usize,
    ) -> usize;
}

fn checked_pixels(width: u32, height: u32) -> Result<usize, PerfError> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(PerfError::SizeOverflow)
}

pub fn hiz_output_cells(width: u32, height: u32) -> Result<usize, PerfError> {
    let out_width = (width as usize)
        .checked_add(1)
        .ok_or(PerfError::SizeOverflow)?
        / 2;
    let out_height = (height as usize)
        .checked_add(1)
        .ok_or(PerfError::SizeOverflow)?
        / 2;
    out_width
        .checked_mul(out_height)
        .ok_or(PerfError::SizeOverflow)
}

pub fn edge_stencil_tiles_4x4(width: u32, height: u32) -> Result<usize, PerfError> {
    let out_width = (width as usize)
        .checked_add(3)
        .ok_or(PerfError::SizeOverflow)?
        / 4;
    let out_height = (height as usize)
        .checked_add(3)
        .ok_or(PerfError::SizeOverflow)?
        / 4;
    out_width
        .checked_mul(out_height)
        .ok_or(PerfError::SizeOverflow)
}

pub fn reduce_hiz_rgba16(
    depth: &[f32],
    width: u32,
    height: u32,
) -> Result<Vec<[u16; 4]>, PerfError> {
    let expected = checked_pixels(width, height)?;
    if depth.len() != expected {
        return Err(PerfError::SizeMismatch {
            expected,
            actual: depth.len(),
        });
    }

    let cells = hiz_output_cells(width, height)?;
    if cells == 0 {
        return Ok(Vec::new());
    }

    let mut raw = vec![0u16; cells * 4];
    let written = unsafe {
        gospol_hiz_reduce_rgba16(depth.as_ptr(), width, height, raw.as_mut_ptr(), raw.len())
    };
    if written != cells {
        return Err(PerfError::OutputRejected);
    }

    Ok(raw.as_chunks::<4>().0.to_vec())
}

pub fn edge_stencil_4x4(
    depth: &[f32],
    width: u32,
    height: u32,
    threshold: f32,
) -> Result<Vec<u8>, PerfError> {
    let expected = checked_pixels(width, height)?;
    if depth.len() != expected {
        return Err(PerfError::SizeMismatch {
            expected,
            actual: depth.len(),
        });
    }

    let tiles = edge_stencil_tiles_4x4(width, height)?;
    if tiles == 0 {
        return Ok(Vec::new());
    }

    let mut mask = vec![0u8; tiles];
    let written = unsafe {
        gospol_edge_stencil_4x4(
            depth.as_ptr(),
            width,
            height,
            threshold,
            mask.as_mut_ptr(),
            mask.len(),
        )
    };
    if written != tiles {
        return Err(PerfError::OutputRejected);
    }
    Ok(mask)
}

pub fn blue_noise_hash(x: u32, y: u32, frame: u32, seed: u32) -> u32 {
    unsafe { gospol_blue_noise_hash(x, y, frame, seed) }
}

pub fn gt_tonemap_rgba8(hdr_rgba: &[[f32; 4]], exposure: f32) -> Result<Vec<[u8; 4]>, PerfError> {
    let pixel_count: u32 = hdr_rgba
        .len()
        .try_into()
        .map_err(|_| PerfError::SizeOverflow)?;
    let mut raw = vec![0u8; hdr_rgba.len() * 4];
    let written = unsafe {
        gospol_gt_tonemap_rgba8(
            hdr_rgba.as_ptr().cast(),
            pixel_count,
            exposure,
            raw.as_mut_ptr(),
            raw.len(),
        )
    };
    if written != hdr_rgba.len() {
        return Err(PerfError::OutputRejected);
    }
    Ok(raw.as_chunks::<4>().0.to_vec())
}

pub fn kawase_blur_rgba32f(
    src_rgba: &[[f32; 4]],
    width: u32,
    height: u32,
    radius: u32,
) -> Result<Vec<[f32; 4]>, PerfError> {
    let expected = checked_pixels(width, height)?;
    if src_rgba.len() != expected {
        return Err(PerfError::SizeMismatch {
            expected,
            actual: src_rgba.len(),
        });
    }
    let mut raw = vec![0.0f32; src_rgba.len() * 4];
    let written = unsafe {
        gospol_kawase_blur_rgba32f(
            src_rgba.as_ptr().cast(),
            width,
            height,
            radius,
            raw.as_mut_ptr(),
            raw.len(),
        )
    };
    if written != src_rgba.len() {
        return Err(PerfError::OutputRejected);
    }
    Ok(raw.as_chunks::<4>().0.to_vec())
}
