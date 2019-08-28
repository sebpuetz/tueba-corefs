use conllx::io::Writer;
use getopts::Options;
use lumberjack::{NegraReader, Tree, WriteTree};
use petgraph::prelude::NodeIndex;
use petgraph::visit::VisitMap;
use petgraph::visit::Visitable;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::{env, process};
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

    let (id_maps, mut sentences) = NegraReader::new(reader).into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut id_maps, mut sentences), t| {
            let mut t = t.or_exit("Failed to read tree.", 1);
            id_maps.push(negra_ids(&mut t));
            sentences.push(t);
            (id_maps, sentences)
        },
    );

    for i in 0..sentences.len() {
        for nt in sentences[i].nonterminals().collect::<Vec<_>>() {
            let comment = if let Some(Some(comment)) = sentences[i][nt]
                .features()
                .and_then(|f| f.get_val("comment"))
            {
                comment.to_owned()
            } else {
                continue;
            };

            for part in comment.split_whitespace() {
                let (sent_id, corefs) = if part.contains("R=coreferential") {
                    let mut coref_parts = part.split('.');
                    coref_parts.next().expect("malformed coref");

                    let sentence_and_node = coref_parts
                        .next()
                        .or_exit("Missing sentence id and node id for coref", 1);
                    resolve_id(&sentences, &id_maps, sentence_and_node)
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
                        let new_coref = format!("({},[{}])]", sent_id, corefs.join(","));
                        // insert concatenated corefs
                        features.insert("coref", Some(format!("{},{}", coref, new_coref)));
                    } else {
                        features.insert(
                            "coref",
                            Some(format!("[({},[{}])]", sent_id, corefs.join(","))),
                        );
                    }
                }
            }
        }
    }

    let mut writer = Writer::new(BufWriter::new(output));
    for mut tree in sentences {
        let terminals = tree.terminals().collect::<Vec<_>>();
        for terminal in terminals {
            // remove auxiliary ids
            tree[terminal].features_mut().remove("id").unwrap().unwrap();
            if !keep {
                tree[terminal].features_mut().remove("comment");
            }
        }
        writer.write_tree(&tree).unwrap();
    }
}

fn resolve_id(
    sentences: &[Tree],
    id_maps: &[HashMap<String, NodeIndex>],
    sentence_and_node: &str,
) -> (String, Vec<String>) {
    let parts = sentence_and_node.split(":").collect::<Vec<_>>();
    if parts.len() != 2 {
        eprintln!("Coreference annotation is expected to be sentence_id:node_id");
        process::exit(1);
    }
    let sentence_id = parts[0].parse::<usize>().expect("Can't parse sentence_id") - 1;
    let nt_id = *id_maps[sentence_id].get(parts[1]).or_exit(
        &format!(
            "No entry for node id {}  in sentence {}.",
            sentence_id, parts[1]
        ),
        1,
    );
    let mut term_ids = Vec::new();
    for descendent in sentences[sentence_id]
        .descendent_terminals(nt_id)
        .collect::<Vec<_>>()
    {
        let id = sentences[sentence_id][descendent]
            .features()
            .and_then(|f| f.get_val("id"))
            .or_exit("Token missing id feature", 1)
            .or_exit("Token missing value for id feature", 1);
        let id = id.parse::<usize>().or_exit("invalid id value.", 1);
        term_ids.push(id.to_string());
    }
    (sentence_id.to_string(), term_ids)
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
        // terminals are sorted, annotate the IDs to prevent sorting twice
        tree[terminal]
            .features_mut()
            .insert("id", Some(i.to_string()));
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
    return negra_id_to_node_idx;
}
