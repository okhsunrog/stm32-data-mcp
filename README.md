# stm32-data-mcp

An MCP (Model Context Protocol) server that exposes [stm32-data](https://github.com/embassy-rs/stm32-data) to LLM tools like Claude Code. Query pin mappings, peripheral info, memory maps, interrupts, DMA channels, register layouts, and documentation for any STM32 microcontroller.

## Setup

### 1. Install

```sh
cargo install --git https://github.com/okhsunrog/stm32-data-mcp --locked
```

### 2. Clone the data

```sh
git clone --depth 1 https://github.com/embassy-rs/stm32-data-generated ~/stm32-data-generated
```

### 3. Add to Claude Code

```sh
claude mcp add stm32-data \
  --transport stdio \
  --scope user \
  --env STM32_DATA_DIR=$HOME/stm32-data-generated \
  -- stm32-data-mcp
```

The server auto-detects whether `STM32_DATA_DIR` points to the repo root or its `data/` subdirectory.

### Updating data

```sh
cd ~/stm32-data-generated && git pull
```

No server restart needed — chip data is loaded on demand.

## Tools

| Tool | Description |
|------|-------------|
| `list_chips` | List/filter chips by family prefix (e.g. "STM32F4", "STM32G4") |
| `get_chip_info` | Chip overview: family, die, cores, memory, packages |
| `get_chip_pinout` | Physical pin positions and signals per package |
| `get_peripheral_pins` | Pin mappings for a peripheral with AF numbers |
| `get_pin_functions` | All alternate functions for a GPIO pin |
| `list_peripherals` | All peripherals with addresses, bus clocks, register blocks |
| `get_peripheral_info` | Detailed peripheral: RCC, clocks, interrupts, DMA |
| `get_register_block` | Register layout with fields, bit positions, descriptions |
| `list_interrupts` | All IRQs with numbers |
| `list_dma_channels` | All DMA channels |
| `get_docs` | Datasheet/reference manual/errata links |
| `get_memory_map` | Flash/RAM regions with addresses, sizes, erase granularity |
| `compare_chips` | Side-by-side comparison of two chips |
| `find_chips_with_peripheral` | Search for chips that have a specific peripheral |

## Data source

All data comes from [embassy-rs/stm32-data-generated](https://github.com/embassy-rs/stm32-data-generated), which is built from [embassy-rs/stm32-data](https://github.com/embassy-rs/stm32-data) — the same data that powers the [Embassy](https://github.com/embassy-rs/embassy) embedded Rust framework.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
