//! Custom materials: a terrain that blends textures from simulation state,
//! and animated water. Presentation only.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use bevy::shader::ShaderRef;

/// Terrain texture layers, in array order.
pub const LAYERS: [&str; 6] = ["grass", "forest", "soil", "dry", "rock", "sand"];

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExt {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub layers: Handle<Image>,
}

impl MaterialExtension for TerrainExt {
    fn fragment_shader() -> ShaderRef {
        "client/shaders/terrain.wgsl".into()
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct WaterExt {
    #[uniform(100)]
    pub strength: f32,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        "client/shaders/water.wgsl".into()
    }
}

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

/// Load terrain textures into one array image with a full CPU-built mip chain,
/// so tiled ground stays crisp up close and calm in the distance.
pub fn build_layer_array(assets_dir: &std::path::Path) -> Image {
    const SIZE: u32 = 1024;
    let mut levels: Vec<Vec<image::RgbaImage>> = vec![];
    for name in LAYERS {
        let path = assets_dir.join(format!("client/textures/terrain/{name}.jpg"));
        let img = image::open(&path).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::from_pixel(SIZE, SIZE, image::Rgba([120, 150, 90, 255])));
        let mut cur = image::imageops::resize(&img, SIZE, SIZE, image::imageops::FilterType::Triangle);
        let mut chain = vec![cur.clone()];
        let mut s = SIZE;
        while s > 1 {
            s /= 2;
            cur = image::imageops::resize(&cur, s, s, image::imageops::FilterType::Triangle);
            chain.push(cur.clone());
        }
        levels.push(chain);
    }
    let mips = levels[0].len() as u32;
    // Layer-major: every mip of layer 0, then every mip of layer 1, ...
    let mut data = vec![];
    for chain in &levels {
        for lvl in chain {
            data.extend_from_slice(lvl.as_raw());
        }
    }
    let mut image = Image::new_uninit(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: LAYERS.len() as u32 },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image
}
