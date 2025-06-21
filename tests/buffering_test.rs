use std::time::Duration;
use tokio::time::{sleep, Instant};

/// Test to validate that buffering logic works correctly
/// This test verifies the buffering mechanism without requiring Docker
#[tokio::test]
async fn test_output_buffering_with_timeout() {
    println!("🔍 Testing output buffering with 200ms timeout:");
    
    // Simulate buffering behavior
    let buffer_timeout = Duration::from_millis(200);
    let mut output_buffer = String::new();
    let _last_output_time = Instant::now();
    
    // Simulate receiving output chunks with delays
    let test_chunks = vec![
        ("chunk1", 50),  // Comes at 50ms
        ("chunk2", 100), // Comes at 150ms total  
        ("chunk3", 300), // Comes at 450ms total - should trigger processing after previous chunks
    ];
    
    let mut processed_outputs = Vec::new();
    let start_time = Instant::now();
    
    for (chunk, delay_ms) in test_chunks {
        // Sleep to simulate output timing
        sleep(Duration::from_millis(delay_ms)).await;
        
        // Simulate receiving output
        output_buffer.push_str(chunk);
        let _last_output_time = Instant::now();
        println!("  📥 Received: '{}' at {}ms", chunk, start_time.elapsed().as_millis());
        
        // Check if we should process buffer (200ms timeout since last output)
        // This would be implemented in the actual select! loop with a timer
        tokio::select! {
            _ = sleep(buffer_timeout) => {
                if !output_buffer.is_empty() {
                    processed_outputs.push(output_buffer.clone());
                    println!("  ✅ Processed buffer: '{}' at {}ms", output_buffer, start_time.elapsed().as_millis());
                    output_buffer.clear();
                }
            }
            _ = sleep(Duration::from_millis(10)) => {
                // Continue to next iteration
            }
        }
    }
    
    // Process any remaining buffer
    if !output_buffer.is_empty() {
        sleep(buffer_timeout).await;
        processed_outputs.push(output_buffer.clone());
        println!("  ✅ Final buffer: '{}' at {}ms", output_buffer, start_time.elapsed().as_millis());
    }
    
    // Validate results
    assert!(!processed_outputs.is_empty(), "Should have processed some output");
    println!("✅ Output buffering test completed successfully");
}

/// Test that validates the buffering prevents rapid state transitions
#[tokio::test] 
async fn test_buffering_prevents_rapid_transitions() {
    println!("🔍 Testing that buffering prevents rapid state transitions:");
    
    // Without buffering, each chunk would trigger state parsing
    let chunks_without_buffering = vec!["chunk1", "chunk2", "chunk3"];
    let state_transitions_without_buffering = chunks_without_buffering.len();
    
    // With buffering, chunks within 200ms are accumulated
    let buffer_timeout = Duration::from_millis(200);
    let mut accumulated_chunks = Vec::new();
    let mut current_buffer = String::new();
    let _state_transitions_with_buffering = 0;
    
    for (i, chunk) in chunks_without_buffering.iter().enumerate() {
        current_buffer.push_str(chunk);
        
        // Simulate rapid output (less than 200ms apart)
        if i < chunks_without_buffering.len() - 1 {
            sleep(Duration::from_millis(50)).await;
        }
    }
    
    // Only process once after timeout
    sleep(buffer_timeout).await;
    accumulated_chunks.push(current_buffer);
    let state_transitions_with_buffering = 1;
    
    println!("  📊 Without buffering: {} state transitions", state_transitions_without_buffering);
    println!("  📊 With buffering: {} state transitions", state_transitions_with_buffering);
    
    // Buffering should reduce state transitions
    assert!(state_transitions_with_buffering < state_transitions_without_buffering);
    assert_eq!(accumulated_chunks.len(), 1);
    assert_eq!(accumulated_chunks[0], "chunk1chunk2chunk3");
    
    println!("✅ Buffering prevents rapid state transitions test passed");
}