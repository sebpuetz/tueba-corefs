# Usage

```Bash
cargo install --git https://github.com/sebpuetz/tueba-corefs
tueba-corefs -i input.export -o output.conll
```

## Input

This assumes `input.export` is in the NEGRA export 4 format and coreferences are annotated as comments in the last column, e.g.
`0` denotes the sentence ID and `500` refers to the node id in the sentence:

`%% R=coreferential.0:500`

Consequentially, sentences can't be shuffled.

## Output

The output is a CONLLX file where the span of each coreferential node is annotated with the list of tokens that the coreference
dominates.

E.g. `coref:[(226,[6,7]),(226,[5,6,7])]|` denotes that the given token is dominated by a node that is coreferential for two
nodes in sentence `226`:
1. a node dominating tokens `6,7`
2. a node dominating tokens `5,6,7`
