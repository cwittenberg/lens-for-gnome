// src/engine/hardware.rs
use std::process::Command;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HardwareStatus {
    pub acceleration_type: String, // "CPU", "GPU", "NPU", "TPU"
    pub device_name: String,
    pub api: String, // "Software", "CUDA", "ROCm", "Vulkan", "OpenVINO", "EdgeTPU", "Metal"
    pub is_hardware_dedicated: bool,
}

pub struct HardwareManager;

impl HardwareManager {
    /// Deep system inspection to detect available AI accelerators
    pub fn detect_hardware() -> HardwareStatus {
        // 1. Check NVIDIA GPU (CUDA)
        if let Ok(output) = Command::new("nvidia-smi").arg("--query-gpu=name").arg("--format=csv,noheader").output() {
            if output.status.success() {
                let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !name.is_empty() {
                    return HardwareStatus {
                        acceleration_type: "GPU".to_string(),
                        device_name: name,
                        api: "CUDA".to_string(),
                        is_hardware_dedicated: true,
                    };
                }
            }
        }

        // 2. Check for AMD / Intel GPUs and NPUs via lspci
        if let Ok(output) = Command::new("lspci").output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
                
                // Check for NPU first (Neural Processing Units on modern laptops)
                if stdout.contains("npu") || stdout.contains("neural processing unit") || stdout.contains("ai engine") {
                    let name = if stdout.contains("amd") { "AMD Ryzen AI NPU" } else { "Intel NPU" };
                    return HardwareStatus {
                        acceleration_type: "NPU".to_string(),
                        device_name: name.to_string(),
                        api: "OpenVINO / ONNX / ROCm".to_string(),
                        is_hardware_dedicated: true,
                    };
                }

                // Check for AMD GPU (Catches both Dedicated VGA and APU Display Controllers)
                if (stdout.contains("vga compatible controller") || stdout.contains("display controller")) && stdout.contains("amd") {
                    return HardwareStatus {
                        acceleration_type: "GPU".to_string(),
                        device_name: "AMD Radeon Graphics".to_string(),
                        api: "ROCm / Vulkan".to_string(),
                        is_hardware_dedicated: true,
                    };
                }

                // Check for Intel Discrete/Integrated GPU
                if (stdout.contains("vga compatible controller") || stdout.contains("display controller")) && stdout.contains("intel") {
                    return HardwareStatus {
                        acceleration_type: "GPU".to_string(),
                        device_name: "Intel Graphics".to_string(),
                        api: "Vulkan / OpenVINO".to_string(),
                        is_hardware_dedicated: true,
                    };
                }
            }
        }

        // 3. Check for Google Coral Edge TPU via lsusb
        if let Ok(output) = Command::new("lsusb").output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if stdout.contains("1a6e:089a") || stdout.contains("global unichip") || stdout.contains("coral") {
                    return HardwareStatus {
                        acceleration_type: "TPU".to_string(),
                        device_name: "Google Coral Edge TPU".to_string(),
                        api: "EdgeTPU".to_string(),
                        is_hardware_dedicated: true,
                    };
                }
            }
        }

        // 4. Check for Apple Silicon (Mac environment edge-case)
        if let Ok(output) = Command::new("uname").arg("-m").output() {
            if output.status.success() {
                let arch = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
                if arch == "arm64" {
                    if let Ok(sys_out) = Command::new("sysctl").arg("-n").arg("machdep.cpu.brand_string").output() {
                        let brand = String::from_utf8_lossy(&sys_out.stdout).to_lowercase();
                        if brand.contains("apple") {
                            return HardwareStatus {
                                acceleration_type: "GPU/NPU".to_string(),
                                device_name: "Apple Silicon Neural Engine".to_string(),
                                api: "Metal".to_string(),
                                is_hardware_dedicated: true,
                            };
                        }
                    }
                }
            }
        }

        // 5. Default fallback to standard CPU Processing
        HardwareStatus {
            acceleration_type: "CPU".to_string(),
            device_name: "Standard Processor".to_string(),
            api: "Software".to_string(),
            is_hardware_dedicated: false,
        }
    }

    /// Determines the amount of LLM layers to offload to the hardware accelerator
    pub fn get_optimal_gpu_layers() -> u32 {
        let hw = Self::detect_hardware();
        
        println!("[HardwareManager] Detected: {} via {} API", hw.device_name, hw.api);
        
        if hw.is_hardware_dedicated && hw.acceleration_type != "CPU" {
            println!("[HardwareManager] Hardware accelerator found. Requesting max GPU offloading (9999 layers).");
            9999 // Send maximum possible layers to the hardware
        } else {
            println!("[HardwareManager] No dedicated AI hardware found. Defaulting to CPU (0 layers).");
            0
        }
    }
}