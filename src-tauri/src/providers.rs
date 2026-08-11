//! Replaceable boundaries for AI, publishing and temporary media storage.
use serde_json::Value;
use std::path::Path;
pub trait AiProvider { fn analyse_image(&self, preview: &Path) -> Result<Value,String>; fn generate_caption(&self, analysis: &Value, style: &str, recent: &[String]) -> Result<String,String>; fn generate_hashtags(&self, analysis: &Value) -> Result<Vec<String>,String>; }
pub struct ClaudeSubscriptionProvider;
impl AiProvider for ClaudeSubscriptionProvider { fn analyse_image(&self,_:&Path)->Result<Value,String>{Err("Claude analysis requires privacy consent and authenticated Claude Code".into())} fn generate_caption(&self,_:&Value,_:&str,_:&[String])->Result<String,String>{Err("Claude is not connected".into())} fn generate_hashtags(&self,_:&Value)->Result<Vec<String>,String>{Err("Claude is not connected".into())} }
pub trait InstagramPublisher { fn create_media(&self, public_url:&str, caption:&str)->Result<String,String>; fn check_media_status(&self, container:&str)->Result<String,String>; fn publish_media(&self, container:&str)->Result<String,String>; }
pub trait TemporaryMediaProvider { fn upload(&self,path:&Path)->Result<(String,String),String>; fn delete(&self,reference:&str)->Result<(),String>; }
pub struct DevelopmentMockPublisher;
impl InstagramPublisher for DevelopmentMockPublisher { fn create_media(&self,_:&str,_:&str)->Result<String,String>{Ok("mock-container".into())} fn check_media_status(&self,_:&str)->Result<String,String>{Ok("MOCK_READY".into())} fn publish_media(&self,_:&str)->Result<String,String>{Err("Development mock mode does not publish to Instagram".into())} }
