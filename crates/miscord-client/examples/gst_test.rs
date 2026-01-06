use anyhow::Result;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

fn main() -> Result<()> {
    gst::init()?;

    // Simpler pipeline without caps filter
    let pipeline_str = "avfvideosrc device-index=0 ! \
                        videoconvert ! \
                        appsink name=sink";

    println!("Creating pipeline: {}", pipeline_str);

    let pipeline = gst::parse::launch(pipeline_str)?
        .downcast::<gst::Pipeline>()
        .unwrap();

    let appsink = pipeline
        .by_name("sink")
        .unwrap()
        .downcast::<gst_app::AppSink>()
        .unwrap();

    // Set appsink to emit signals and drop old buffers
    appsink.set_property("emit-signals", true);
    appsink.set_property("sync", false);
    appsink.set_property("max-buffers", 1u32);
    appsink.set_property("drop", true);

    println!("Setting pipeline to PLAYING...");
    pipeline.set_state(gst::State::Playing)?;

    // Wait for state change
    let bus = pipeline.bus().unwrap();
    for msg in bus.iter_timed(gst::ClockTime::from_seconds(5)) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Error(err) => {
                println!("ERROR: {} ({:?})", err.error(), err.debug());
                pipeline.set_state(gst::State::Null)?;
                return Err(anyhow::anyhow!("Pipeline error"));
            }
            MessageView::StateChanged(s) => {
                if s.src().map(|s| s == &pipeline).unwrap_or(false) {
                    println!("Pipeline state: {:?} -> {:?}", s.old(), s.current());
                    if s.current() == gst::State::Playing {
                        println!("Pipeline is now PLAYING");
                        break;
                    }
                }
            }
            MessageView::Eos(_) => {
                println!("EOS on bus!");
                break;
            }
            _ => {}
        }
    }

    println!("Attempting to pull samples...");
    for i in 0..10 {
        println!("  Pulling sample {}...", i);
        match appsink.try_pull_sample(gst::ClockTime::from_seconds(2)) {
            Some(sample) => {
                let buffer = sample.buffer().unwrap();
                println!("  Frame {}: {} bytes", i, buffer.size());
            }
            None => {
                if appsink.is_eos() {
                    println!("  EOS reached at frame {}", i);
                    break;
                }
                println!("  No sample available at frame {}", i);
            }
        }
    }

    pipeline.set_state(gst::State::Null)?;
    println!("Done");
    Ok(())
}
