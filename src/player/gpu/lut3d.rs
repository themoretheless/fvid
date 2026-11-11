use crate::color::{Grade, Interpolation, Lut3d, ShaderLook};
use eframe::wgpu;
use std::sync::Arc;

/// The tables bound for a graded plane picture, and the grade that filled them:
/// either the 3D grid of nodes at binding 5, or the three byte tables at binding
/// 6, whichever the grade itself reads.
///
/// A grade with nothing to look up still needs both bound — a two-node grid and
/// an empty row of bytes the shader never reads, because `grid.w` says the
/// lookup is off.
pub(super) struct Grid {
    /// The grade these textures mirror, kept so the frame after this one can be
    /// told it reads the same colour without comparing a megabyte of nodes. A
    /// grade whose tables the shader cannot carry is `None` here and stays on
    /// the route that takes both steps on the CPU.
    pub(super) source: Option<Arc<Grade>>,
    pub(super) texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    /// The byte tables' one row of 256 texels. Fixed in size, so it is made once
    /// and only ever rewritten, and the bind group that holds it stays good.
    pub(super) look_texture: wgpu::Texture,
    pub(super) look_view: wgpu::TextureView,
    pub(super) look_size: usize,
    pub(super) look_linear: bool,
    pub(super) look_params: [f32; 12],
    pub(super) table_texture: wgpu::Texture,
    pub(super) table_view: wgpu::TextureView,
    /// Edge length the grid texture was made for, and a count of how often one
    /// has replaced another: the bind group holds the view, so a new texture
    /// needs a new bind group and nothing else does.
    pub(super) size: usize,
    pub(super) epoch: usize,
    /// Edge length, read mode, which table and the on switch, as the uniform
    /// carries them.
    pub(super) params: [f32; 4],
}

/// The two-node grid bound when a picture has nothing to grade.
pub(super) const GRID_OFF: [f32; 4] = [2.0, 0.0, 0.0, 0.0];

/// The number the shader names the byte tables with in `grid.z`.
pub(super) const TABLES: f32 = 1.0;

/// The number the shader matches a read mode against: 0 snaps to a node, 1
/// blends the eight around it, 2 walks the four of a tetrahedron.
pub(super) fn grid_mode(interpolation: Interpolation) -> f32 {
    match interpolation {
        Interpolation::Nearest => 0.0,
        Interpolation::Trilinear => 1.0,
        Interpolation::Tetrahedral => 2.0,
    }
}


impl Grid {
    /// True when the bound texture already mirrors exactly this grade, which is
    /// what lets a repaint of the same picture skip both the table copy and the
    /// uniform that names it.
    pub(super) fn holds(&self, grade: Option<&Arc<Grade>>) -> bool {
        match (&self.source, grade) {
            (Some(held), Some(want)) => Arc::ptr_eq(held, want),
            (None, None) => true,
            _ => false,
        }
    }

    /// The grid bound while a picture has nothing to look up, with the shader's
    /// switch set so it never reads either table.
    pub(super) fn empty(device: &wgpu::Device) -> Self {
        let (texture, view) = make_grid(device, 2);
        let (table_texture, table_view) = make_tables(device);
        let (look_texture, look_view) = make_grid(device, 2);
        Self {
            source: None,
            texture,
            view,
            look_texture,
            look_view,
            look_size: 2,
            look_linear: false,
            look_params: [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0],
            table_texture,
            table_view,
            size: 2,
            params: GRID_OFF,
            epoch: 0,
        }
    }

    /// Point the bound tables at `grade`'s own lookup. A grid of the same edge
    /// length is written into the texture that is already there, so the bind
    /// group that holds it stays good and only a different size — a new
    /// `--grid`, say, or a grade that reads bytes rather than nodes — costs a
    /// new texture and a rebinding.
    pub(super) fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grade: Option<&Arc<Grade>>) {
        if self.holds(grade) {
            return;
        }
        let look = grade.and_then(|grade| grade.shader_look()).or_else(|| {
            grade.and_then(|g| match g.shader_stages() {
                crate::color::ShaderStages::Chain {
                    conversion: crate::color::Lut::Three(cube),
                    look: Some(_),
                } if g.is_gpu_grade() => Some(ShaderLook::Grid(cube.clone())),
                _ => None,
            })
        });
        let second = grade.and_then(|g| match g.shader_stages() {
            crate::color::ShaderStages::Chain {
                look: Some(crate::color::Lut::Three(cube)),
                ..
            } => Some(cube),
            _ => None,
        });
        self.look_params = [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];
        if let Some(cube) = second {
            if cube.size != self.look_size || self.look_linear {
                (self.look_texture, self.look_view) = make_grid(device, cube.size);
                self.look_size = cube.size;
                self.look_linear = false;
                self.epoch += 1;
            }
            upload_grid(queue, &self.look_texture, cube.size, &cube.data);
            self.look_params = [
                cube.size as f32,
                grid_mode(grade.unwrap().interpolation()),
                0.0,
                1.0,
                cube.domain_min[0],
                cube.domain_min[1],
                cube.domain_min[2],
                0.0,
                cube.domain_max[0],
                cube.domain_max[1],
                cube.domain_max[2],
                0.0,
            ];
        }
        if let Some(crate::color::ShaderStages::Chain {
            look: Some(crate::color::Lut::One(line)),
            ..
        }) = grade
            .filter(|g| g.is_gpu_grade())
            .map(|g| g.shader_stages())
        {
            let size = line.len();
            if self.look_size != size || !self.look_linear {
                (self.look_texture, self.look_view) = make_line(device, size);
                self.look_size = size;
                self.look_linear = true;
                self.epoch += 1;
            }
            let extent = self.look_texture.size();
            let row = (extent.width as usize * 16).next_multiple_of(256);
            let mut bytes =
                vec![0u8; row * extent.height as usize * extent.depth_or_array_layers as usize];
            for i in 0..size {
                let offset = (i / extent.width as usize) * row + (i % extent.width as usize) * 16;
                for channel in 0..3 {
                    bytes[offset + channel * 4..offset + channel * 4 + 4]
                        .copy_from_slice(&line.data[channel][i].to_le_bytes());
                }
            }
            queue.write_texture(
                self.look_texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row as u32),
                    rows_per_image: Some(extent.height),
                },
                extent,
            );
            self.look_params = [
                size as f32,
                0.0,
                1.0,
                1.0,
                line.domain_min[0],
                line.domain_min[1],
                line.domain_min[2],
                0.0,
                line.domain_max[0],
                line.domain_max[1],
                line.domain_max[2],
                0.0,
            ];
        }
        let grid = match &look {
            Some(ShaderLook::Grid(cube)) => Some(cube),
            _ => None,
        };
        let size = grid.map_or(2, |cube| cube.size);
        if size != self.size {
            let (texture, view) = make_grid(device, size);
            self.texture = texture;
            self.view = view;
            self.size = size;
            self.epoch += 1;
        }
        let identity_grid;
        let nodes: &[[f32; 3]] = match grid {
            Some(cube) => &cube.data,
            None => {
                identity_grid = Lut3d::identity(2);
                &identity_grid.data
            }
        };
        upload_grid(queue, &self.texture, size, nodes);
        if let Some(ShaderLook::Tables(tables)) = &look {
            // One texel per input code, holding each channel's own answer in its
            // own component, so the shader reads the exact bytes `paint` writes.
            let mut tabled = [0u8; 4 * 256];
            for code in 0..256 {
                for channel in 0..3 {
                    tabled[code * 4 + channel] = tables[channel][code];
                }
                tabled[code * 4 + 3] = 255;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.table_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &tabled,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * 256),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 256,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.source = grade.cloned();
        // A picture with no table to read leaves the switch off, so it pays for
        // the bindings and not for the lookup.
        self.params = match (&grade, &look) {
            (Some(grade), Some(ShaderLook::Grid(cube))) => {
                [cube.size as f32, grid_mode(grade.interpolation()), 0.0, 1.0]
            }
            (Some(grade), Some(ShaderLook::Tables(_))) => {
                [1.0, grid_mode(grade.interpolation()), TABLES, 1.0]
            }
            _ => GRID_OFF,
        };
    }
}

fn make_line(device: &wgpu::Device, size: usize) -> (wgpu::Texture, wgpu::TextureView) {
    let width = size.min(128);
    let height = size.div_ceil(width).min(128);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid float 1D look"),
        size: wgpu::Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: size.div_ceil(width * height) as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn upload_grid(queue: &wgpu::Queue, texture: &wgpu::Texture, size: usize, nodes: &[[f32; 3]]) {
    // One `rgba32float` texel per node, in the table's own order: red
    // fastest, so the node (r, g, b) lands at texel (r, g, b), which is how
    // the shader indexes it. `write_texture` copies through a staging buffer
    // whose rows are 256-byte aligned, so a node row is padded out to that.
    const TEXEL: usize = 4 * std::mem::size_of::<f32>();
    let packed = size * TEXEL;
    let row = packed.next_multiple_of(256);
    let mut bytes = vec![0u8; row * size * size];
    for (index, node) in nodes.iter().enumerate() {
        let from = index * TEXEL;
        let to = (from / packed) * row + from % packed;
        for (channel, value) in [node[0], node[1], node[2], 1.0].iter().enumerate() {
            let at = to + channel * std::mem::size_of::<f32>();
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row as u32),
            rows_per_image: Some(size as u32),
        },
        wgpu::Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: size as u32,
        },
    );
}

/// A 3D texture with one `rgba32float` texel per node of a `size`³ grid.
fn make_grid(device: &wgpu::Device, size: usize) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid grade grid"),
        size: wgpu::Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: size as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// A 256×1 row of `rgba8unorm`, one texel per input code of the three byte
/// tables. The bytes the CPU stores are the bytes the shader loads, so neither
/// route rounds a second time.
fn make_tables(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid grade tables"),
        size: wgpu::Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
