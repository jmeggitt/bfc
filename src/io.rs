use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Read};
use std::mem::replace;

use getopts::Matches;
use tempfile::NamedTempFile;
use std::collections::HashMap;
use regex::Regex;

use crate::{bfir, executable_name, execution, link_object_file, llvm, peephole, strip_executable};
use crate::bfir::{AstNode, Position};
use crate::diagnostics::{Info, Level};
use crate::execution::ExecutionState;

pub trait IncludesResolver<R: Read> {
    fn try_include(&mut self, include: String) -> Result<BufReader<R>, Info>;
}

pub struct PreProcessor {
    defines: HashMap<String, String>,
    visits: Vec<String>,
    reader: BufReader<File>,
    file_stack: Vec<BufReader<File>>,
    buffer: String,
    buffer_idx: usize,
}

impl PreProcessor {

    pub fn read_line(&mut self) -> io::Result<usize> {
        // TODO: Make regexes initialize using lazy static
        let includes = Regex::new(r#"^#\s*include\s+[<"]([^<>"])[>"]\s*$"#).unwrap();


        loop {
            let read_len = self.reader.read_line(&mut self.buffer)?;
            self.buffer_idx = 0;

            if self.buffer.starts_with('#') {

                if let Some(capture) = includes.captures(&self.buffer) {
                    let file = capture.get(0).unwrap().as_str();
                    let include_file = String::from(&file[1..file.len() - 1]);
                    println!("Including file: {}", &include_file);
                    let old = replace(&mut self.reader, BufReader::new(File::open(include_file)?));
                    self.file_stack.push(old);
                }

                continue
            }

            match (read_len, self.file_stack.is_empty()) {
                (0, false) => self.reader = self.file_stack.pop().unwrap(),
                (x, _) => return Ok(x),
            }
        }
    }
}

impl Read for PreProcessor {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        unimplemented!()
    }
}


pub struct ErrorContext {
    pub line_col: (u64, u64),
    pub line: String,
    pub file: String,
}

pub struct SingleFileReader {
    inner: BufReader<File>,
    path: String,
}

impl SingleFileReader {
    pub fn new(path: impl Into<String>) -> Result<Self, Info> {
        let path = path.into();
        match File::open(path.clone()) {
            Ok(file) => Ok(SingleFileReader {
                inner: BufReader::new(file),
                path,
            }),
            Err(e) => Err(Info {
                level: Level::Error,
                filename: Some(path),
                message: format!("{}", e),
                position: None,
                source: None,
                line_col: None,
            }),
        }
    }

    pub fn parse(&mut self) -> Result<Vec<AstNode>, Vec<Info>> {
        let mut offset = 0;
        let mut linenum = 0;
        let mut buffer = String::default();

        let mut instructions = Vec::new();
        let mut stack = Vec::new();

        let mut errors = Vec::new();
        loop {
            let line_len = match self.inner.read_line(&mut buffer) {
                Ok(0) => break,
                Ok(v) => v,
                Err(e) => {
                    errors.push(Info {
                        level: Level::Error,
                        filename: Some(self.path.clone()),
                        message: format!("{}", e),
                        position: None,
                        source: None,
                        line_col: None,
                    });
                    return Err(errors);
                }
            };
            if let Err(e) = bfir::parse_inner(&buffer, offset, &mut instructions, &mut stack) {
                errors.push(Info {
                    level: Level::Error,
                    filename: Some(self.path.clone()),
                    message: e.message,
                    position: Some(e.position),
                    source: Some(buffer.clone()),
                    line_col: Some((linenum, (e.position.start - offset) as u64)),
                });
            }

            offset += line_len;
            linenum += 1;
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(instructions)
        }
    }

    pub fn get_err_context(&mut self, mut idx: u64) -> Result<ErrorContext, Info> {
        if let Err(e) = self.inner.seek(SeekFrom::Start(0)) {
            return Err(Info {
                level: Level::Error,
                filename: Some(self.path.clone()),
                message: format!("{}", e),
                position: None,
                source: None,
                line_col: None,
            });
        }

        let mut line = 0;
        let mut buffer = String::default();

        loop {
            match self.inner.read_line(&mut buffer) {
                Ok(0) => return Err(Info::error("Reached EOF before error context could be found")),
                Ok(len) => {
                    if len as u64 > idx {
                        return Ok(ErrorContext {
                            line_col: (line, idx),
                            line: buffer,
                            file: self.path.clone(),
                        });
                    }
                    idx -= len as u64;
                    line += 1;
                }
                Err(e) => return Err(Info {
                    level: Level::Error,
                    filename: Some(self.path.clone()),
                    message: format!("{}", e),
                    position: None,
                    source: None,
                    line_col: None,
                }),
            }
        }
    }
}


// TODO: return a Vec<Info> that may contain warnings or errors,
// instead of printing in lots of different places here.
pub fn compile_file(matches: &Matches) -> Result<(), Vec<Info>> {
    let path: &String = &matches.free[0];

    let mut reader = match SingleFileReader::new(path) {
        Ok(v) => v,
        Err(e) => return Err(vec![e])
    };

    let mut instrs = reader.parse()?;
    let mut errors = Vec::new();
    let mut unformatted_warnings = Vec::new();

    let opt_level = matches.opt_str("opt").unwrap_or_else(|| String::from("2"));
    if opt_level != "0" {
        let pass_specification = matches.opt_str("passes");
        let (opt_instrs, warnings) = peephole::optimize(instrs, &pass_specification);
        instrs = opt_instrs;
        unformatted_warnings = warnings;
    }

    if matches.opt_present("dump-ir") {
        for instr in &instrs {
            println!("{}", instr);
        }
        return Ok(());
    }

    let (state, execution_warning) = if opt_level == "2" {
        execution::execute(&instrs, execution::max_steps())
    } else {
        let mut init_state = execution::ExecutionState::initial(&instrs[..]);
        // TODO: this will crash on the empty program.
        init_state.start_instr = Some(&instrs[0]);
        (init_state, None)
    };

    if let Some(execution_warning) = execution_warning {
        unformatted_warnings.push(execution_warning);
    }

    for warning in unformatted_warnings {
        let info = match warning.position {
            Some(Position {start, ..}) => {
                match reader.get_err_context(start as u64) {
                    Ok(ErrorContext { line_col, line, file }) => Info {
                        level: Level::Warning,
                        filename: Some(file),
                        message: warning.message,
                        position: warning.position,
                        source: Some(line),
                        line_col: Some(line_col),
                    },
                    Err(e) => e,
                }
            }
            None => Info::warn(warning.message),
        };

        errors.push(info);
    }

    if let Err(e) = handoff_to_llvm(path, matches, &instrs[..], &state) {
        errors.push(e);
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(())
}

pub fn handoff_to_llvm(outfile: &str, matches: &Matches, instrs: &[AstNode], state: &ExecutionState) -> Result<(), Info> {
    llvm::init_llvm();
    let target_triple = matches.opt_str("target");
    let mut llvm_module = llvm::compile_to_module(outfile, target_triple.clone(), &instrs, &state);

    if matches.opt_present("dump-llvm") {
        let llvm_ir_cstr = llvm_module.to_cstring();
        let llvm_ir = String::from_utf8_lossy(llvm_ir_cstr.as_bytes());
        println!("{}", llvm_ir);
        return Ok(());
    }

    let llvm_opt_raw = matches
        .opt_str("llvm-opt")
        .unwrap_or_else(|| "3".to_owned());
    let mut llvm_opt = llvm_opt_raw.parse::<i64>().unwrap_or(3);
    if llvm_opt < 0 || llvm_opt > 3 {
        // TODO: warn on unrecognised input.
        llvm_opt = 3;
    }


    llvm::optimise_ir(&mut llvm_module, llvm_opt);

    // Compile the LLVM IR to a temporary object file.
    // let object_file = convert_io_error(NamedTempFile::new())?;
    let object_file = match NamedTempFile::new() {
        Ok(v) => v,
        Err(e) => return Err(Info::error(format!("{}", e))),
    };

    let obj_file_path = object_file.path().to_str().expect("path not valid utf-8");
    llvm::write_object_file(&mut llvm_module, &obj_file_path)?;

    let output_name = executable_name(outfile);
    link_object_file(&obj_file_path, &output_name, target_triple)?;

    let strip_opt = matches.opt_str("strip").unwrap_or_else(|| "yes".to_owned());
    if strip_opt == "yes" {
        strip_executable(&output_name)?
    }

    Ok(())
}

#[cfg(test)]
mod tests {


}
