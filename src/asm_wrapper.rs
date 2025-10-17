//! Wrapper for external asm6502 assembler library
//!
//! Wraps the asm6502 library from GitHub for inline assembly.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

#![allow(dead_code)]

use asm6502::{Assembler6502, AsmError as Asm6502Error};

#[derive(Debug)]
pub enum AsmError {
    Asm(String),
}

impl From<Asm6502Error> for AsmError {
    fn from(e: Asm6502Error) -> Self {
        // Extract detailed error information from asm6502 error
        let error_msg = format!("{:?}", e);
        AsmError::Asm(error_msg)
    }
}

pub struct Assembler6502Wrapper {
    assembler: Assembler6502,
}

impl Assembler6502Wrapper {
    pub fn new() -> Self {
        Assembler6502Wrapper {
            assembler: Assembler6502::new(),
        }
    }

    /// Assemble source into raw bytes with enhanced error reporting
    pub fn assemble_bytes(&mut self, src: &str) -> Result<Vec<u8>, AsmError> {
        self.assembler.reset();

        match self.assembler.assemble_bytes(src) {
            Ok(bytes) => Ok(bytes),
            Err(e) => {
                // Try to extract line information from the error and source
                let error_msg = self.format_assembly_error(&e, src);
                Err(AsmError::Asm(error_msg))
            }
        }
    }

    /// Assemble source into a C64 PRG file (with $0801 load address)
    pub fn assemble_prg(&mut self, src: &str) -> Result<Vec<u8>, AsmError> {
        let binary = self.assemble_bytes(src)?;

        // Prepend PRG header ($01 $08 - load address $0801)
        let mut prg = vec![0x01, 0x08];
        prg.extend_from_slice(&binary);

        Ok(prg)
    }

    /// Format assembly error with line number and instruction context
    fn format_assembly_error(&self, error: &Asm6502Error, source: &str) -> String {
        let error_string = format!("{:?}", error);

        // Try to extract line information by analyzing the error
        // Common patterns in asm6502 errors
        if error_string.contains("Unknown instruction") ||
            error_string.contains("unknown mnemonic") {
            return self.find_error_context(source, &error_string, "instruction");
        }

        if error_string.contains("Invalid") ||
            error_string.contains("Parse error") ||
            error_string.contains("Expected") {
            return self.find_error_context(source, &error_string, "syntax");
        }

        if error_string.contains("Undefined") {
            return self.find_error_context(source, &error_string, "undefined");
        }

        if error_string.contains("Long-branch") {
            return format!("Assembly error: {}\n\nNote: This may be caused by branch instructions that are out of range.\nTry using absolute JMP instructions for long distances.", error_string);
        }

        // Default: return the error with source line count
        let line_count = source.lines().count();
        format!("Assembly error: {}\n(Source has {} lines)", error_string, line_count)
    }

    /// Find error context by searching through source lines
    fn find_error_context(&self, source: &str, error_msg: &str, error_type: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();

        // Try to extract a keyword from the error message
        let keyword = self.extract_keyword_from_error(error_msg);

        if let Some(kw) = keyword {
            // Search for the keyword in source
            for (line_num, line) in lines.iter().enumerate() {
                let line_trimmed = line.trim();
                if line_trimmed.contains(&kw) && !line_trimmed.starts_with(';') {
                    return format!(
                        "Assembly error at line {}: {}\n\nLine {}: {}\n\nError: {}",
                        line_num + 1,
                        error_type,
                        line_num + 1,
                        line.trim(),
                        error_msg
                    );
                }
            }
        }

        // If we can't find specific line, return error with context
        format!("Assembly error ({}): {}\n\nTotal lines in source: {}",
                error_type, error_msg, lines.len())
    }

    /// Extract a keyword from error message (instruction name, label, etc)
    fn extract_keyword_from_error(&self, error_msg: &str) -> Option<String> {
        // Try to find quoted text first
        if let Some(start) = error_msg.find('"') {
            if let Some(end) = error_msg[start + 1..].find('"') {
                return Some(error_msg[start + 1..start + 1 + end].to_string());
            }
        }

        // Try to find text after "instruction:" or "mnemonic:"
        if let Some(pos) = error_msg.find("instruction:") {
            let remainder = &error_msg[pos + 12..].trim();
            if let Some(word) = remainder.split_whitespace().next() {
                return Some(word.to_string());
            }
        }

        if let Some(pos) = error_msg.find("mnemonic:") {
            let remainder = &error_msg[pos + 9..].trim();
            if let Some(word) = remainder.split_whitespace().next() {
                return Some(word.to_string());
            }
        }

        // Try to extract label names (pattern: word followed by "undefined" or "not found")
        if error_msg.contains("Undefined") || error_msg.contains("not found") {
            let words: Vec<&str> = error_msg.split_whitespace().collect();
            for i in 0..words.len().saturating_sub(1) {
                if words[i + 1] == "Undefined" || words[i + 1] == "undefined" ||
                    words[i + 1] == "not" {
                    return Some(words[i].to_string());
                }
            }
        }

        None
    }
}

impl Default for Assembler6502Wrapper {
    fn default() -> Self {
        Self::new()
    }
}
