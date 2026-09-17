use crate::analyzer::face_detector::BBox;
use crate::analyzer::gpu::create_session_with_fallback;
use image::RgbImage;
use ndarray::Array4;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Value;

const INPUT_SIZE: u32 = 256;

pub struct LandmarkModel {
    session: Session,
    input_name: String,
    output_name: String,
}

impl LandmarkModel {
    pub fn new(model_path: &str) -> anyhow::Result<Self> {
        let session =
            create_session_with_fallback(model_path, GraphOptimizationLevel::Level3, Some(4))?;

        let input_name = session.inputs()[0].name().to_string();
        let output_name = session.outputs()[0].name().to_string();

        Ok(Self {
            session,
            input_name,
            output_name,
        })
    }

    pub fn predict(
        &mut self,
        rgb: &RgbImage,
        bbox: &BBox,
    ) -> anyhow::Result<Option<Vec<(f32, f32)>>> {
        let margin = 0.3;
        let bbox_w = bbox.x2 - bbox.x1;
        let bbox_h = bbox.y2 - bbox.y1;
        let pad_x = bbox_w * margin;
        let pad_y = bbox_h * margin;

        let x1 = (bbox.x1 - pad_x).max(0.0) as u32;
        let y1 = (bbox.y1 - pad_y).max(0.0) as u32;
        let side = ((bbox_w + pad_x * 2.0).max(bbox_h + pad_y * 2.0)) as u32;
        let side = side
            .min(rgb.width().saturating_sub(x1))
            .min(rgb.height().saturating_sub(y1))
            .max(1);

        let cropped = image::imageops::crop_imm(rgb, x1, y1, side, side).to_image();
        let resized = image::imageops::resize(
            &cropped,
            INPUT_SIZE,
            INPUT_SIZE,
            image::imageops::FilterType::Triangle,
        );

        let mut tensor = Array4::<f32>::zeros((1, INPUT_SIZE as usize, INPUT_SIZE as usize, 3));
        for (x, y, pixel) in resized.enumerate_pixels() {
            let [r, g, b] = pixel.0;
            tensor[[0, y as usize, x as usize, 0]] = r as f32 / 255.0;
            tensor[[0, y as usize, x as usize, 1]] = g as f32 / 255.0;
            tensor[[0, y as usize, x as usize, 2]] = b as f32 / 255.0;
        }

        let input_value = Value::from_array(tensor)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => input_value])
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let (_, raw) = outputs[self.output_name.as_str()].try_extract_tensor::<f32>()?;

        let landmarks = raw
            .chunks(3)
            .map(|xyz| {
                let x = x1 as f32 + (xyz[0] / INPUT_SIZE as f32) * side as f32;
                let y = y1 as f32 + (xyz[1] / INPUT_SIZE as f32) * side as f32;
                (x, y)
            })
            .collect();

        Ok(Some(landmarks))
    }
}
