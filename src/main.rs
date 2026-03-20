use std::{fmt::Write as _, path::PathBuf, sync::Arc};

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use stm32_data_serde::{
    Chip,
    chip::{Memory, core::peripheral::rcc::KernelClock},
};

// ── Tool parameter types ──

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListChipsParams {
    #[schemars(description = "Filter by chip family prefix, e.g. \"STM32F4\", \"STM32L0\"")]
    family_filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ChipParam {
    #[schemars(description = "Chip name, e.g. \"STM32F411CE\"")]
    chip: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CompareChipsParams {
    #[schemars(description = "First chip name, e.g. \"STM32F411CE\"")]
    chip_a: String,
    #[schemars(description = "Second chip name, e.g. \"STM32F411RE\"")]
    chip_b: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct FindChipsParams {
    #[schemars(
        description = "Peripheral name to search for, e.g. \"OPAMP3\", \"FDCAN2\", \"USB_OTG_HS\""
    )]
    peripheral: String,
    #[schemars(description = "Optional family prefix filter, e.g. \"STM32G4\", \"STM32H7\"")]
    family_filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PeripheralParam {
    #[schemars(description = "Chip name, e.g. \"STM32F411CE\"")]
    chip: String,
    #[schemars(description = "Peripheral name, e.g. \"SPI1\", \"USART2\", \"I2C1\"")]
    peripheral: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PinParam {
    #[schemars(description = "Chip name, e.g. \"STM32F411CE\"")]
    chip: String,
    #[schemars(description = "GPIO pin name, e.g. \"PA0\", \"PB5\"")]
    pin: String,
}

// ── Server ──

#[derive(Debug, Clone)]
struct Stm32DataServer {
    data_dir: Arc<PathBuf>,
    registers_dir: Arc<PathBuf>,
    chip_names: Arc<Vec<String>>,
    tool_router: ToolRouter<Self>,
}

impl Stm32DataServer {
    fn new(data_dir: PathBuf, registers_dir: PathBuf) -> Self {
        let mut chip_names: Vec<String> = std::fs::read_dir(&data_dir)
            .expect("Cannot read data directory")
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".json").map(|s| s.to_string())
            })
            .collect();
        chip_names.sort();

        eprintln!(
            "stm32-data-mcp: loaded {} chip names from {}",
            chip_names.len(),
            data_dir.display()
        );

        Self {
            data_dir: Arc::new(data_dir),
            registers_dir: Arc::new(registers_dir),
            chip_names: Arc::new(chip_names),
            tool_router: Self::tool_router(),
        }
    }

    fn load_chip(&self, name: &str) -> Result<Chip, String> {
        let path = self.data_dir.join(format!("{name}.json"));
        let data = std::fs::read_to_string(&path).map_err(|_| {
            format!("Chip '{name}' not found. Use list_chips to see available chips.")
        })?;
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse chip data: {e}"))
    }

    fn load_registers(&self, kind: &str, version: &str) -> Result<serde_json::Value, String> {
        let filename = format!("{kind}_{version}.json");
        let path = self.registers_dir.join(&filename);
        let data = std::fs::read_to_string(&path)
            .map_err(|_| format!("Register file '{filename}' not found."))?;
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse register data: {e}"))
    }
}

fn fmt_size(bytes: u32) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{}B", bytes)
    }
}

fn fmt_kernel_clock(kc: &KernelClock) -> String {
    match kc {
        KernelClock::Clock(s) => s.clone(),
        KernelClock::Mux(field) => format!("{}.{}", field.register, field.field),
    }
}

fn mem_summary(memory: &[Vec<Memory>]) -> (u32, u32) {
    let mut flash = 0u32;
    let mut ram = 0u32;
    for bank in memory {
        for region in bank {
            use stm32_data_serde::chip::memory::Kind;
            match region.kind {
                Kind::Flash if !region.name.contains("OTP") => flash += region.size,
                Kind::Ram => ram += region.size,
                _ => {}
            }
        }
    }
    (flash, ram)
}

#[tool_router(router = tool_router)]
impl Stm32DataServer {
    #[tool(
        name = "list_chips",
        description = "List available STM32 chips. Optionally filter by family prefix like \"STM32F4\" or \"STM32L0\"."
    )]
    async fn list_chips(&self, Parameters(params): Parameters<ListChipsParams>) -> String {
        let filter = params.family_filter.map(|f| f.to_uppercase());

        let chips: Vec<&str> = self
            .chip_names
            .iter()
            .filter(|name| match &filter {
                Some(f) => name.starts_with(f.as_str()),
                None => true,
            })
            .map(|s| s.as_str())
            .collect();

        if chips.is_empty() {
            return "No chips found matching the filter.".to_string();
        }

        format!("{} chips found:\n{}", chips.len(), chips.join("\n"))
    }

    #[tool(
        name = "get_chip_info",
        description = "Get overview info for a chip: family, line, die, device ID, core(s), memory regions (flash/RAM sizes), available packages, and number of peripherals/interrupts/DMA channels."
    )]
    async fn get_chip_info(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!(
            "Chip: {}\nFamily: {}\nLine: {}\nDie: {}\nDevice ID: {:#06x}\n",
            chip.name, chip.family, chip.line, chip.die, chip.device_id
        );

        for core in &chip.cores {
            let _ = writeln!(out, "\nCore: {}", core.name);
            if let Some(bits) = core.nvic_priority_bits {
                let _ = writeln!(out, "  NVIC priority bits: {bits}");
            }
            let _ = writeln!(out, "  Peripherals: {}", core.peripherals.len());
            let _ = writeln!(out, "  Interrupts: {}", core.interrupts.len());
            let _ = writeln!(out, "  DMA channels: {}", core.dma_channels.len());
        }

        out.push_str("\nMemory:\n");
        for bank in &chip.memory {
            for region in bank {
                let _ = write!(
                    out,
                    "  {} ({:?}): {} at {:#010x}",
                    region.name,
                    region.kind,
                    fmt_size(region.size),
                    region.address
                );
                if let Some(settings) = &region.settings {
                    if settings.erase_size > 0 {
                        let _ = write!(out, ", erase={}", fmt_size(settings.erase_size));
                    }
                    let _ = write!(out, ", write={}", fmt_size(settings.write_size));
                }
                out.push('\n');
            }
        }

        out.push_str("\nPackages:\n");
        for pkg in &chip.packages {
            let _ = writeln!(
                out,
                "  {} ({}, {} pins)",
                pkg.name,
                pkg.package,
                pkg.pins.len()
            );
        }

        out
    }

    #[tool(
        name = "get_chip_pinout",
        description = "Get physical pin positions and signals for a chip's packages. Returns package type and pin-to-signal mapping."
    )]
    async fn get_chip_pinout(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!(
            "Chip: {} (family: {}, line: {})\n",
            chip.name, chip.family, chip.line
        );

        for pkg in &chip.packages {
            let _ = writeln!(out, "\nPackage: {} ({})", pkg.name, pkg.package);
            let _ = writeln!(out, "{} pins:", pkg.pins.len());
            for pin in &pkg.pins {
                let _ = writeln!(out, "  Pin {}: {}", pin.position, pin.signals.join(", "));
            }
        }

        out
    }

    #[tool(
        name = "get_peripheral_pins",
        description = "Get pin mappings for a specific peripheral (e.g. SPI1, USART2, I2C1) on a chip, including alternate function numbers."
    )]
    async fn get_peripheral_pins(&self, Parameters(params): Parameters<PeripheralParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let periph_name = params.peripheral.to_uppercase();

        for core in &chip.cores {
            for periph in &core.peripherals {
                if periph.name == periph_name {
                    if periph.pins.is_empty() {
                        return format!(
                            "Peripheral {} exists on {} but has no pin mappings.",
                            periph_name, chip.name
                        );
                    }
                    let mut out = format!("Peripheral {} on {}:\n\n", periph_name, chip.name);
                    for p in &periph.pins {
                        match p.af {
                            Some(af) => {
                                let _ = writeln!(out, "  {} -> {} (AF{})", p.pin, p.signal, af);
                            }
                            None => {
                                let _ = writeln!(out, "  {} -> {}", p.pin, p.signal);
                            }
                        }
                    }
                    return out;
                }
            }
        }

        let available: Vec<&str> = chip.cores[0]
            .peripherals
            .iter()
            .filter(|p| !p.pins.is_empty())
            .map(|p| p.name.as_str())
            .collect();

        format!(
            "Peripheral '{}' not found on {}.\nAvailable peripherals with pins: {}",
            periph_name,
            chip.name,
            available.join(", ")
        )
    }

    #[tool(
        name = "get_pin_functions",
        description = "Get all alternate functions and peripheral connections for a specific GPIO pin (e.g. PA0, PB5) on a chip."
    )]
    async fn get_pin_functions(&self, Parameters(params): Parameters<PinParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let pin_name = params.pin.to_uppercase();
        let mut functions: Vec<String> = Vec::new();

        for core in &chip.cores {
            for periph in &core.peripherals {
                for p in &periph.pins {
                    if p.pin == pin_name {
                        let af_str = match p.af {
                            Some(af) => format!(" (AF{af})"),
                            None => String::new(),
                        };
                        functions.push(format!("  {}_{}{af_str}", periph.name, p.signal));
                    }
                }
            }
        }

        if functions.is_empty() {
            return format!(
                "Pin '{}' has no peripheral mappings on {}.",
                pin_name, chip.name
            );
        }

        functions.sort();
        format!(
            "Pin {} on {} — {} functions:\n\n{}",
            pin_name,
            chip.name,
            functions.len(),
            functions.join("\n")
        )
    }

    #[tool(
        name = "list_peripherals",
        description = "List all peripherals on a chip with their addresses, bus clocks, and register block types."
    )]
    async fn list_peripherals(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("Peripherals on {} ({}):\n\n", chip.name, chip.cores.len());

        for core in &chip.cores {
            for periph in &core.peripherals {
                let _ = write!(out, "  {} @ {:#010x}", periph.name, periph.address);
                if let Some(regs) = &periph.registers {
                    let _ = write!(out, " [{}:{}:{}]", regs.kind, regs.version, regs.block);
                }
                if let Some(rcc) = &periph.rcc {
                    let _ = write!(out, " bus={}", rcc.bus_clock);
                }
                if !periph.pins.is_empty() {
                    let _ = write!(out, " ({} pins)", periph.pins.len());
                }
                out.push('\n');
            }
        }

        out
    }

    #[tool(
        name = "get_peripheral_info",
        description = "Get detailed info for a specific peripheral: address, register block, RCC enable/reset fields, bus clock, and pin count."
    )]
    async fn get_peripheral_info(&self, Parameters(params): Parameters<PeripheralParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let periph_name = params.peripheral.to_uppercase();

        for core in &chip.cores {
            for periph in &core.peripherals {
                if periph.name == periph_name {
                    let mut out = format!(
                        "Peripheral {} on {} (core: {}):\n\n",
                        periph.name, chip.name, core.name
                    );
                    let _ = writeln!(out, "  Address: {:#010x}", periph.address);

                    if let Some(regs) = &periph.registers {
                        let _ = writeln!(
                            out,
                            "  Registers: kind={}, version={}, block={}",
                            regs.kind, regs.version, regs.block
                        );
                    }

                    if let Some(rcc) = &periph.rcc {
                        let _ = writeln!(out, "  Bus clock: {}", rcc.bus_clock);
                        let _ = writeln!(
                            out,
                            "  Kernel clock: {}",
                            fmt_kernel_clock(&rcc.kernel_clock)
                        );
                        let _ = writeln!(
                            out,
                            "  RCC enable: {}.{}",
                            rcc.enable.register, rcc.enable.field
                        );
                        if let Some(reset) = &rcc.reset {
                            let _ =
                                writeln!(out, "  RCC reset: {}.{}", reset.register, reset.field);
                        }
                        let _ = writeln!(out, "  Stop mode: {:?}", rcc.stop_mode);
                    }

                    let _ = writeln!(out, "  Pins: {}", periph.pins.len());

                    if !periph.interrupts.is_empty() {
                        let _ = writeln!(out, "  Interrupts:");
                        for irq in &periph.interrupts {
                            let _ = writeln!(out, "    {} -> {}", irq.signal, irq.interrupt);
                        }
                    }

                    if !periph.dma_channels.is_empty() {
                        let _ = writeln!(out, "  DMA channels:");
                        for dma in &periph.dma_channels {
                            let _ = write!(out, "    {}", dma.signal);
                            if let Some(req) = dma.request {
                                let _ = write!(out, " (request={})", req);
                            }
                            out.push('\n');
                        }
                    }

                    return out;
                }
            }
        }

        format!("Peripheral '{}' not found on {}.", periph_name, chip.name)
    }

    #[tool(
        name = "list_interrupts",
        description = "List all interrupts on a chip with their IRQ numbers."
    )]
    async fn list_interrupts(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("Interrupts on {}:\n\n", chip.name);

        for core in &chip.cores {
            if chip.cores.len() > 1 {
                let _ = writeln!(out, "Core: {}", core.name);
            }
            let mut interrupts = core.interrupts.clone();
            interrupts.sort_by_key(|i| i.number);
            for irq in &interrupts {
                let _ = writeln!(out, "  {:3}: {}", irq.number, irq.name);
            }
        }

        out
    }

    #[tool(
        name = "list_dma_channels",
        description = "List all DMA channels on a chip."
    )]
    async fn list_dma_channels(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("DMA channels on {}:\n\n", chip.name);

        for core in &chip.cores {
            for ch in &core.dma_channels {
                let _ = writeln!(
                    out,
                    "  {} (controller: {}, channel: {})",
                    ch.name, ch.dma, ch.channel
                );
            }
        }

        out
    }

    #[tool(
        name = "get_docs",
        description = "Get documentation links for a chip: reference manuals, datasheets, programming manuals, errata, and application notes."
    )]
    async fn get_docs(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("Documentation for {}:\n\n", chip.name);

        for doc in &chip.docs {
            let _ = writeln!(out, "  [{}] {}", doc.r#type, doc.title);
            let _ = writeln!(out, "    {}: {}", doc.name, doc.url);
        }

        out
    }

    #[tool(
        name = "get_memory_map",
        description = "Get detailed memory map for a chip: flash banks/regions, RAM, OTP, with addresses, sizes, and erase/write granularity."
    )]
    async fn get_memory_map(&self, Parameters(params): Parameters<ChipParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("Memory map for {}:\n\n", chip.name);

        for (i, bank) in chip.memory.iter().enumerate() {
            if chip.memory.len() > 1 {
                let _ = writeln!(out, "Bank {}:", i);
            }
            for region in bank {
                let _ = writeln!(out, "  {} ({:?}):", region.name, region.kind);
                let _ = writeln!(
                    out,
                    "    Address: {:#010x} - {:#010x}",
                    region.address,
                    region.address + region.size
                );
                let _ = writeln!(
                    out,
                    "    Size: {} ({} bytes)",
                    fmt_size(region.size),
                    region.size
                );
                if let Some(settings) = &region.settings {
                    if settings.erase_size > 0 {
                        let _ = writeln!(out, "    Erase size: {}", fmt_size(settings.erase_size));
                    }
                    let _ = writeln!(out, "    Write size: {}", fmt_size(settings.write_size));
                    let _ = writeln!(out, "    Erase value: {:#04x}", settings.erase_value);
                }
            }
        }

        out
    }

    #[tool(
        name = "get_register_block",
        description = "Get the register layout for a peripheral on a chip: registers with offsets, fields with bit positions, and enum values. Looks up the register block type from the chip's peripheral info."
    )]
    async fn get_register_block(&self, Parameters(params): Parameters<PeripheralParam>) -> String {
        let chip = match self.load_chip(&params.chip.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let periph_name = params.peripheral.to_uppercase();

        let mut reg_info = None;
        for core in &chip.cores {
            for periph in &core.peripherals {
                if periph.name == periph_name {
                    reg_info = periph.registers.as_ref();
                    break;
                }
            }
        }

        let reg_info = match reg_info {
            Some(r) => r,
            None => {
                return format!(
                    "Peripheral '{}' not found on {}, or it has no register block.",
                    periph_name, chip.name
                );
            }
        };

        let regs = match self.load_registers(&reg_info.kind, &reg_info.version) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut out = format!(
            "Registers for {} on {} ({}_{}, block={}):\n\n",
            periph_name, chip.name, reg_info.kind, reg_info.version, reg_info.block
        );

        let regs_obj = match regs.as_object() {
            Some(o) => o,
            None => return "Invalid register data format.".to_string(),
        };

        let block_key = format!("block/{}", reg_info.block);
        if let Some(block) = regs_obj.get(&block_key).and_then(|v| v.as_object()) {
            if let Some(desc) = block.get("description").and_then(|v| v.as_str()) {
                let _ = writeln!(out, "{desc}\n");
            }
            if let Some(items) = block.get("items").and_then(|v| v.as_array()) {
                out.push_str("Registers:\n");
                for item in items {
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let offset = item
                        .get("byte_offset")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let desc = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let access = item.get("access").and_then(|v| v.as_str());
                    let bit_size = item.get("bit_size").and_then(|v| v.as_u64());

                    let _ = write!(out, "  {:#06x} {name}", offset);
                    if let Some(bs) = bit_size {
                        let _ = write!(out, " ({bs}bit)");
                    }
                    if let Some(acc) = access {
                        let _ = write!(out, " [{acc}]");
                    }
                    if !desc.is_empty() {
                        let _ = write!(out, " — {desc}");
                    }
                    out.push('\n');

                    if let Some(fs_name) = item.get("fieldset").and_then(|v| v.as_str()) {
                        let fs_key = format!("fieldset/{fs_name}");
                        if let Some(fieldset) = regs_obj.get(&fs_key).and_then(|v| v.as_object())
                            && let Some(fields) = fieldset.get("fields").and_then(|v| v.as_array())
                        {
                            for field in fields {
                                let fname =
                                    field.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let foff = field
                                    .get("bit_offset")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let fsize =
                                    field.get("bit_size").and_then(|v| v.as_u64()).unwrap_or(1);
                                let fdesc = field
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");

                                if fsize == 1 {
                                    let _ = write!(out, "    [{foff}] {fname}");
                                } else {
                                    let _ = write!(
                                        out,
                                        "    [{foff}:{end}] {fname}",
                                        end = foff + fsize - 1
                                    );
                                }
                                if !fdesc.is_empty() {
                                    let _ = write!(out, " — {fdesc}");
                                }
                                out.push('\n');
                            }
                        }
                    }
                }
            }
        }

        out
    }

    #[tool(
        name = "compare_chips",
        description = "Compare two STM32 chips side-by-side: memory (flash/RAM), peripherals present in one but not the other, packages, and core info."
    )]
    async fn compare_chips(&self, Parameters(params): Parameters<CompareChipsParams>) -> String {
        let chip_a = match self.load_chip(&params.chip_a.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let chip_b = match self.load_chip(&params.chip_b.to_uppercase()) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut out = format!("Comparison: {} vs {}\n\n", chip_a.name, chip_b.name);

        let _ = writeln!(out, "Family: {} vs {}", chip_a.family, chip_b.family);
        let _ = writeln!(out, "Line: {} vs {}", chip_a.line, chip_b.line);
        let _ = writeln!(out, "Die: {} vs {}", chip_a.die, chip_b.die);

        out.push_str("\n--- Memory ---\n");
        let (flash_a, ram_a) = mem_summary(&chip_a.memory);
        let (flash_b, ram_b) = mem_summary(&chip_b.memory);
        let _ = writeln!(
            out,
            "  Flash: {} vs {}",
            fmt_size(flash_a),
            fmt_size(flash_b)
        );
        let _ = writeln!(out, "  RAM: {} vs {}", fmt_size(ram_a), fmt_size(ram_b));

        out.push_str("\n--- Packages ---\n");
        let _ = writeln!(
            out,
            "  {}: {}",
            chip_a.name,
            chip_a
                .packages
                .iter()
                .map(|p| format!("{} ({}pin)", p.package, p.pins.len()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let _ = writeln!(
            out,
            "  {}: {}",
            chip_b.name,
            chip_b
                .packages
                .iter()
                .map(|p| format!("{} ({}pin)", p.package, p.pins.len()))
                .collect::<Vec<_>>()
                .join(", ")
        );

        use std::collections::BTreeSet;
        let periphs_a: BTreeSet<&str> = chip_a.cores[0]
            .peripherals
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let periphs_b: BTreeSet<&str> = chip_b.cores[0]
            .peripherals
            .iter()
            .map(|p| p.name.as_str())
            .collect();

        let only_a: Vec<&&str> = periphs_a.difference(&periphs_b).collect();
        let only_b: Vec<&&str> = periphs_b.difference(&periphs_a).collect();
        let common = periphs_a.intersection(&periphs_b).count();

        let _ = writeln!(out, "\n--- Peripherals ---");
        let _ = writeln!(out, "  Common: {} peripherals", common);
        if !only_a.is_empty() {
            let _ = writeln!(
                out,
                "  Only on {}: {}",
                chip_a.name,
                only_a.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
            );
        }
        if !only_b.is_empty() {
            let _ = writeln!(
                out,
                "  Only on {}: {}",
                chip_b.name,
                only_b.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
            );
        }
        if only_a.is_empty() && only_b.is_empty() {
            let _ = writeln!(out, "  Both chips have identical peripheral sets.");
        }

        let _ = writeln!(
            out,
            "\n--- Interrupts ---\n  {}: {}\n  {}: {}",
            chip_a.name,
            chip_a.cores[0].interrupts.len(),
            chip_b.name,
            chip_b.cores[0].interrupts.len()
        );

        let _ = writeln!(
            out,
            "\n--- DMA channels ---\n  {}: {}\n  {}: {}",
            chip_a.name,
            chip_a.cores[0].dma_channels.len(),
            chip_b.name,
            chip_b.cores[0].dma_channels.len()
        );

        out
    }

    #[tool(
        name = "find_chips_with_peripheral",
        description = "Search for chips that have a specific peripheral (e.g. OPAMP3, FDCAN2, USB_OTG_HS, LTDC). Optionally filter by family prefix."
    )]
    async fn find_chips_with_peripheral(
        &self,
        Parameters(params): Parameters<FindChipsParams>,
    ) -> String {
        let periph_name = params.peripheral.to_uppercase();
        let family_filter = params.family_filter.map(|f| f.to_uppercase());

        let candidates: Vec<&str> = self
            .chip_names
            .iter()
            .filter(|name| match &family_filter {
                Some(f) => name.starts_with(f.as_str()),
                None => true,
            })
            .map(|s| s.as_str())
            .collect();

        let mut matches: Vec<String> = Vec::new();

        for chip_name in &candidates {
            let chip = match self.load_chip(chip_name) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let has_periph = chip
                .cores
                .iter()
                .any(|core| core.peripherals.iter().any(|p| p.name == periph_name));

            if has_periph {
                matches.push(chip_name.to_string());
            }
        }

        if matches.is_empty() {
            return format!(
                "No chips found with peripheral '{}'{}.",
                periph_name,
                family_filter
                    .map(|f| format!(" in family {f}"))
                    .unwrap_or_default()
            );
        }

        format!(
            "{} chips with {}:\n{}",
            matches.len(),
            periph_name,
            matches.join("\n")
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Stm32DataServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions("STM32 chip data server. Query pin mappings, peripheral info, memory maps, interrupts, DMA channels, and documentation for any STM32 microcontroller.")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // STM32_DATA_DIR should point to the stm32-data-generated repo root or its data/ subdirectory.
    // We look for data/chips/ and data/registers/ under it, or chips/ and registers/ directly.
    let base_dir = std::env::var("STM32_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join("stm32-data-generated")
        });

    let (chips_dir, registers_dir) = if base_dir.join("data/chips").is_dir() {
        // Repo root: stm32-data-generated/data/chips/
        (base_dir.join("data/chips"), base_dir.join("data/registers"))
    } else if base_dir.join("chips").is_dir() {
        // data/ subdirectory directly
        (base_dir.join("chips"), base_dir.join("registers"))
    } else {
        eprintln!(
            "Error: could not find chip data in: {}\n\
             Expected either {{dir}}/data/chips/ or {{dir}}/chips/ to exist.\n\
             Clone https://github.com/embassy-rs/stm32-data-generated and set STM32_DATA_DIR to the repo path.",
            base_dir.display()
        );
        std::process::exit(1);
    };

    if !registers_dir.is_dir() {
        eprintln!(
            "Warning: registers directory not found: {}\nRegister block queries will not work.",
            registers_dir.display()
        );
    }

    let server = Stm32DataServer::new(chips_dir, registers_dir);
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    server.serve(transport).await?.waiting().await?;
    Ok(())
}
