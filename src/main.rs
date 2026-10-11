use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::BufWriter;
use std::io::Write;
use std::iter;
use std::process::Command;
use std::process::Stdio;
use std::str::FromStr;
use tempfile::NamedTempFile;
use tempfile::TempPath;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os();
    let _ = args.next();
    let in_path = args.next().unwrap();
    let out_path = args.next().unwrap();
    let command = args.next().unwrap();
    let command_args = args.collect::<Vec<_>>();

    let content = fs::read_to_string(&in_path).expect("Failed to read input");
    let candidate = content.parse()?;

    assert!(
        interesting(&command, &command_args, &candidate)?.0,
        "input does not reproduce"
    );

    let mut cfg: Cfg = candidate.into();
    let mut pos = 0;
    let mut changed = false;
    let mut candidates_tested = Vec::with_capacity(cfg.lines.len());
    loop {
        let candidates = candidates(&cfg, pos);
        let mut hit = None;
        let mut test_count = 0;

        for candidate in candidates.into_iter() {
            test_count = test_count + 1;
            if let (true, candidate_path) = interesting(&command, &command_args, &candidate.1)? {
                hit = Some((candidate, candidate_path));
                break;
            }
        }

        candidates_tested.push(test_count);

        if let Some((hit, hit_path)) = hit {
            (pos, cfg, changed) = (hit.0, hit.1.into(), true);

            std::fs::copy(&hit_path, &out_path)?;

            println!("{} lines", cfg.lines.len());
        } else if changed {
            (pos, changed) = (0, false);
        } else {
            break;
        }
    }

    println!();
    println!("Reduction steps: {}", candidates_tested.len());
    while let Some(&lowest) = candidates_tested.iter().min() {
        let old_len = candidates_tested.len();
        candidates_tested = candidates_tested.iter().filter(|c| **c != lowest).map(|n| *n).collect();
        let count_lowest_iters = old_len - candidates_tested.len();
        println!("Number of steps testing {lowest} candidates: {count_lowest_iters}");
    }

    Ok(())
}

fn interesting(
    command: &OsString,
    command_args: &[OsString],
    cfg: &CfgCandidate,
) -> Result<(bool, TempPath), Box<dyn Error>> {
    let mut command = Command::new(command);
    let mut buffer = BufWriter::new(NamedTempFile::new()?);

    buffer.write_all(cfg.to_string().as_bytes())?;

    let path = buffer.into_inner()?.into_temp_path();

    let res = command
        .args(command_args)
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();

    Ok((!res, path))
}

fn candidates(cfg: &Cfg, start_position: usize) -> Vec<(usize, CfgCandidate<'_>)> {
    let mut candidates = Vec::new();

    for (i, line) in cfg.lines.iter().enumerate().skip(start_position) {
        let (before, after) = (&cfg.lines[..i], &cfg.lines[i + 1..]);

        match line {
            Line::Sideeffect(_) => {
                let candidate = CfgCandidate::Single(before, None, after);
                candidates.push((i, candidate))
            }
            Line::C(n1, n2, n3) => {
                for target in [n2, n3] {
                    let candidate =
                        CfgCandidate::Single(before, Some(Line::B(*n1, *target)), after);
                    candidates.push((i, candidate))
                }
            }
            Line::Switch(nodes) => {
                if nodes.len() > 4 {
                    for k in 2..nodes.len() {
                        let new_line = Line::Switch({
                            let mut tmp = nodes.clone();
                            tmp.remove(k);
                            tmp
                        });

                        let candidate = CfgCandidate::Single(before, Some(new_line), after);
                        candidates.push((i, candidate))
                    }
                }
                let n1 = nodes[0];
                for target in nodes.iter().skip(1) {
                    let candidate = CfgCandidate::Single(before, Some(Line::B(n1, *target)), after);
                    candidates.push((i, candidate))
                }
            }
            Line::B(a, x) if *a != Node::Entry && a != x => {
                // Remove block a by sending its predecessors straight to x.

                let mut candidate_lines = Vec::new();
                for line in iter::chain(before, after) {
                    match line {
                        Line::Sideeffect(n) if n == a => (),
                        Line::Sideeffect(n) => candidate_lines.push(Line::Sideeffect(*n)),
                        Line::B(n1, n2) => {
                            candidate_lines.push(Line::B(*n1, if n2 == a { *x } else { *n2 }))
                        }
                        Line::C(n1, n2, n3) => candidate_lines.push(Line::C(
                            *n1,
                            if n2 == a { *x } else { *n2 },
                            if n3 == a { *x } else { *n3 },
                        )),
                        Line::Switch(nodes) => {
                            let n1 = nodes[0];
                            let mut replaced_nodes: Vec<_> = nodes
                                .iter()
                                .map(|&n| if n == *a { *x } else { n })
                                .collect();
                            replaced_nodes[0] = n1;
                            candidate_lines.push(Line::Switch(replaced_nodes));
                        }
                    }
                }
                candidates.push((i, CfgCandidate::All(candidate_lines)))
            }
            _ => (),
        }
    }

    candidates
}

struct Cfg {
    lines: Vec<Line>,
}

enum CfgCandidate<'a> {
    Single(&'a [Line], Option<Line>, &'a [Line]),
    All(Vec<Line>),
}

#[derive(Clone)]
enum Line {
    Sideeffect(Node),
    B(Node, Node),
    C(Node, Node, Node),
    Switch(Vec<Node>),
}

#[derive(Copy, Clone, PartialEq)]
enum Node {
    Entry,
    Normal(u32),
}

impl FromStr for CfgCandidate<'_> {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lines = s
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()?;

        Ok(CfgCandidate::All(lines))
    }
}

impl From<Cfg> for CfgCandidate<'_> {
    fn from(candidate: Cfg) -> Self {
        CfgCandidate::All(candidate.lines)
    }
}

impl From<CfgCandidate<'_>> for Cfg {
    fn from(candidate: CfgCandidate) -> Self {
        match candidate {
            CfgCandidate::Single(before, new_line, after) => Cfg {
                lines: iter::chain(before, iter::chain(new_line.iter(), after))
                    .cloned()
                    .collect(),
            },
            CfgCandidate::All(lines) => Cfg { lines },
        }
    }
}

impl FromStr for Line {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut fragments = s.split(" ");

        let line = match fragments.next().unwrap() {
            "sideeffect" => Line::Sideeffect(fragments.next().unwrap().parse()?),
            "b" => Line::B(
                fragments.next().unwrap().parse()?,
                fragments.next().unwrap().parse()?,
            ),
            "c" => Line::C(
                fragments.next().unwrap().parse()?,
                fragments.next().unwrap().parse()?,
                fragments.next().unwrap().parse()?,
            ),
            "switch" => Line::Switch(fragments.map(str::parse).collect::<Result<Vec<_>, _>>()?),
            _ => unreachable!(),
        };

        Ok(line)
    }
}

impl FromStr for Node {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "entry" {
            return Ok(Node::Entry);
        }

        match s.split_at(1) {
            ("n", num) => Ok(Node::Normal(num.parse()?)),
            _ => Err(format!("Unexpected node: {s}").into()),
        }
    }
}

impl fmt::Display for CfgCandidate<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CfgCandidate::Single(before, new_line, after) => {
                for line in iter::chain(*before, iter::chain(new_line, *after)) {
                    writeln!(f, "{line}")?;
                }
            }
            CfgCandidate::All(lines) => {
                for line in lines {
                    writeln!(f, "{line}")?;
                }
            }
        }

        Ok(())
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Line::Sideeffect(n) => write!(f, "sideeffect {n}"),
            Line::B(n1, n2) => write!(f, "b {n1} {n2}"),
            Line::C(n1, n2, n3) => write!(f, "c {n1} {n2} {n3}"),
            Line::Switch(nodes) => {
                write!(f, "switch")?;
                for node in nodes {
                    write!(f, " {node}")?
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Node::Entry => write!(f, "entry"),
            Node::Normal(num) => write!(f, "n{num}"),
        }
    }
}
