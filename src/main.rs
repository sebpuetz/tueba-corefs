use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::{env, process};

use conllx::io::Writer;
use getopts::Options;
use itertools::Itertools;
use lumberjack::{NegraReader, Tree, WriteTree};
use petgraph::prelude::NodeIndex;
use petgraph::visit::{VisitMap, Visitable};
use stdinout::OrExit;

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} FILE [options]", program);
    print!("{}", opts.usage(&brief));
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("i", "input", "set input file name", "NAME");
    opts.optopt("o", "output", "set output file name", "NAME");
    opts.optopt("k", "keep_comments", "keep all NEGRA comments", "NAME");
    opts.optflag("h", "help", "print this help menu");
    let matches = opts.parse(&args[1..]).or_exit("Couldn't parse args", 1);
    if matches.opt_present("h") {
        print_usage(&program, opts);
        return;
    }
    let input_path = matches.opt_str("i").or_exit("Missing input file name", 1);
    let output_path = matches.opt_str("o").or_exit("Missing output file name", 1);
    let keep = matches.opt_str("k").is_some();

    let input = File::open(input_path).or_exit("Can't open input file.", 1);
    let reader = BufReader::new(input);
    let output = File::create(output_path).or_exit("Can't open output file.", 1);

    // collect trees from corpus and get mapping from negra_id to node_idx
    let (id_maps, mut sentences) = NegraReader::new(reader).fold(
        (Vec::new(), Vec::new()),
        |(mut id_maps, mut sentences), t| {
            let mut t = t.or_exit("Failed to read tree.", 1);
            id_maps.push(negra_ids(&mut t));
            sentences.push(t);
            (id_maps, sentences)
        },
    );

    for i in 0..sentences.len() {
        for nt in sentences[i].graph().node_indices().collect::<Vec<_>>() {
            // only nodes with comments can have coref annotations
            let comment = if let Some(Some(comment)) = sentences[i][nt]
                .features()
                .and_then(|f| f.get_val("comment"))
            {
                comment.to_owned()
            } else {
                continue;
            };

            for part in comment.split_whitespace() {
                // comments also contain annotations for anaphora, typos etc.
                let (sent_id, corefs) = if part.contains("R=coreferential") {
                    let mut coref_parts = part.split('.');
                    coref_parts.next().expect("malformed coref");

                    let sentence_and_node = coref_parts
                        .next()
                        .or_exit("Missing sentence id and node id for coref", 1)
                        .split(':')
                        .collect::<Vec<_>>();
                    if sentence_and_node.len() != 2 {
                        eprintln!("Coreference annotation is expected to be sentence_id:node_id");
                        process::exit(1);
                    }
                    let sentence_id = sentence_and_node[0].parse::<usize>().unwrap() - 1;
                    // map negra_id to node_idx
                    let negra_id = sentence_and_node[1];
                    let nt_id = *id_maps[sentence_id].get(negra_id).or_exit(
                        &format!(
                            "No entry for node id {}  in sentence {}.",
                            negra_id, sentence_id
                        ),
                        1,
                    );

                    let spanned_tokens = sentences[sentence_id][nt_id]
                        .span()
                        .into_iter()
                        .map(|idx| idx.to_string())
                        .join(",");
                    (sentence_id, spanned_tokens)
                } else {
                    continue;
                };

                for terminal in sentences[i].descendent_terminals(nt).collect::<Vec<_>>() {
                    let features = sentences[i][terminal].features_mut();
                    // get previously added coref
                    if let Some(coref) = features.remove("coref") {
                        let coref = coref.or_exit("Missing coref feature", 1);
                        // save to slice, last idx is "]"
                        let coref = &coref[..coref.len() - 1];
                        let new_coref = format!("({},[{}])]", sent_id, corefs);
                        // insert concatenated corefs
                        features.insert("coref", Some(format!("{},{}", coref, new_coref)));
                    } else {
                        features.insert("coref", Some(format!("[({},[{}])]", sent_id, corefs)));
                    }
                }
            }
        }
    }

    let mut writer = Writer::new(BufWriter::new(output));
    for mut tree in sentences {
        let terminals = tree.terminals().collect::<Vec<_>>();
        for terminal in terminals {
            if !keep {
                tree[terminal].features_mut().remove("comment");
            }
        }
        writer
            .write_tree(&tree)
            .or_exit("Unable to write sentence.", 1);
    }
}

fn negra_ids(tree: &mut Tree) -> HashMap<String, NodeIndex> {
    let mut terminals: Vec<NodeIndex> = tree.terminals().collect::<Vec<_>>();
    terminals.sort_by(|node1, node2| tree[*node1].span().cmp(&tree[*node2].span()));

    let mut visit_map = tree.graph().visit_map();
    let mut negra_id = 500;

    let mut negra_id_to_node_idx = HashMap::new();
    negra_id_to_node_idx.insert("0".into(), tree.root());
    let mut queue = VecDeque::new();

    // find nonterminals that don't dominate other nonterminals
    for (i, &terminal) in terminals.iter().enumerate() {
        // i+1 because root gets 0
        negra_id_to_node_idx.insert((i + 1).to_string(), terminal);
        visit_map.visit(terminal);
        if let Some((parent, _)) = tree.parent(terminal) {
            if tree.children(parent).all(|(n, _)| tree[n].is_terminal()) {
                queue.push_back(parent);
            }
        }
    }

    // assign numbers from 500 onwards to nonterminals
    let mut nts = Vec::new();
    while let Some(node) = queue.pop_front() {
        if !visit_map.visit(node) || tree.root() == node {
            continue;
        }
        negra_id_to_node_idx.insert(negra_id.to_string(), node);
        nts.push(node);
        negra_id += 1;
        if let Some((parent, _)) = tree.parent(node) {
            if tree.children(parent).all(|(n, _)| visit_map.is_visited(&n)) {
                queue.push_front(parent);
            }
        }
    }
    negra_id_to_node_idx
}
