use image::RgbImage;
use ndarray::Array4;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Value;

#[derive(Clone)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

pub struct FaceDetector {
    session: Session,
    width: u32,
    height: u32,
}

impl FaceDetector {
    pub fn new(model_path: &str) -> anyhow::Result<Self> {
        let session = Session::builder().map_err(|e| anyhow::anyhow!("{e}"))?;
        let session = crate::analyzer::gpu::with_gpu_providers(session)?;
        let session = session
            .with_intra_threads(4)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .commit_from_file(model_path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self {
            session,
            width: 320,
            height: 240,
        })
    }

    pub fn detect(&mut self, rgb: &RgbImage) -> anyhow::Result<Option<BBox>> {
        let (orig_w, orig_h) = (rgb.width() as f32, rgb.height() as f32);
        let resized = image::imageops::resize(
            rgb,
            self.width,
            self.height,
            image::imageops::FilterType::Triangle,
        );

        let mut tensor = Array4::<f32>::zeros((1, 3, self.height as usize, self.width as usize));
        for (x, y, pixel) in resized.enumerate_pixels() {
            let [r, g, b] = pixel.0;
            tensor[[0, 0, y as usize, x as usize]] = (r as f32 - 127.0) / 128.0;
            tensor[[0, 1, y as usize, x as usize]] = (g as f32 - 127.0) / 128.0;
            tensor[[0, 2, y as usize, x as usize]] = (b as f32 - 127.0) / 128.0;
        }

        let input_value = Value::from_array(tensor)?;
        let outputs = self
            .session
            .run(ort::inputs!["input" => input_value])
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let (_, scores) = outputs["scores"].try_extract_tensor::<f32>()?;
        let (_, boxes) = outputs["boxes"].try_extract_tensor::<f32>()?;

        let n = scores.len() / 2;
        let mut max0 = f32::MIN;
        let mut max1 = f32::MIN;
        for i in 0..n {
            max0 = max0.max(scores[i * 2]);
            max1 = max1.max(scores[i * 2 + 1]);
        }
        eprintln!("max score[0]={max0:.4} max score[1]={max1:.4}");

        let mut best: Option<(f32, BBox)> = None;
        for i in 0..n {
            let face_score = scores[i * 2];
            if face_score < 0.7 {
                continue;
            }
            if best.as_ref().is_some_and(|(s, _)| *s >= face_score) {
                continue;
            }
            best = Some((
                face_score,
                BBox {
                    x1: boxes[i * 4] * orig_w,
                    y1: boxes[i * 4 + 1] * orig_h,
                    x2: boxes[i * 4 + 2] * orig_w,
                    y2: boxes[i * 4 + 3] * orig_h,
                },
            ));
        }
        eprintln!(
            "max face score this frame: {:?}",
            best.as_ref().map(|(s, _)| s)
        );
        Ok(best.map(|(_, b)| b))
    }
}
