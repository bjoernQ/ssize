# Stack Size

This uses [stack-sizes](https://crates.io/crates/stack-sizes) to show the stack size (and code size) of functions. All the limitations mentioned in `stack-sizes` apply here.

```
Usage: ssize [OPTIONS]

Options:
      --bin <BIN>                    Build only the specified binary
      --example <NAME>               Build only the specified example
      --features <FEATURES>          Space-separated list of features to activate
      --all-features                 Activate all available features
      --min-stack <MIN_STACK>        Only show functions whose stack size is greater or equals to this
      --out-override <OUT_OVERRIDE>  Override the path of the resulting ELF - use if for some reason it's not found
  -h, --help                         Print help
  -V, --version                      Print version
```

It will build your code in release mode and show the result. e.g.
```
❯ ssize --example ble --features=ble --min-stack=1000
   Compiling esp32c3-hal v0.11.0 (C:\projects\esp\esp-hal\esp32c3-hal)
   Compiling esp-wifi v0.1.0 (C:\projects\esp\esp-wifi\esp-wifi)
   Compiling examples-util v0.1.0 (C:\projects\esp\esp-wifi\examples-util)
   Compiling examples-esp32c3 v0.1.0 (C:\projects\esp\esp-wifi\examples-esp32c3)
    Finished release [optimized + debuginfo] target(s) in 1.22s
Code  Stack Name
  750  2480 esp_wifi::common_adapter::chip_specific::phy_enable::h3a9078ab35430dbd
 1476  1712 esp_wifi::compat::common::vsnprintf::h7c7b528c48398010
 2412  1552 <esp_backtrace::arch::TrapFrame as core::fmt::Debug>::fmt::he4fe50a0acb68cb8
 2702  1280 main
  358  1136 sprintf
  512  1072 esp_wifi::ble::btdm::notify_host_recv::hb626d7ca78118a3e
 1230  1040 bleps::attribute_server::AttributeServer::do_work_with_notification::h2c790f15e3d7dcd0
```
