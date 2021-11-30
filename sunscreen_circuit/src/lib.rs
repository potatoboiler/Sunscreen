#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! This crate contains the types for manipulating the intermediate representation
//! for Sunscreen's compiler backend.

mod error;
mod literal;
mod operation;
mod validation;

use petgraph::{
    algo::is_isomorphic_matching,
    algo::toposort,
    algo::tred::*,
    graph::{Graph, NodeIndex},
    stable_graph::{Edges, Neighbors, StableGraph},
    visit::{IntoNeighbors, IntoNodeIdentifiers},
    Directed, Direction,
};
use serde::{Deserialize, Serialize};

pub use error::*;
pub use literal::*;
pub use operation::*;
use IRTransform::*;
use TransformNodeIndex::*;

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
/**
 * There 3 primary FHE schemes in use today: BFV, CKKS, and TFHE. BFV is generally the best choice for algorithms
 * requiring exact arithmetic on integers easily expressed as addition and multiplication. CKKS is generally best
 * suited for approximate arithmetic easily expressed as addition and multiplication. TFHE is generally best
 * when it's non-trivial to express the majority of operations and multiplications and additions. A prescribed
 * sequence of homomorphic operations on ciphertexts is referred to as a circuit.
 *
 * Homomorphic operations in all three schemes introduce noise into the resulting ciphertext. If a ciphertext
 * contains too much noise, decryption will fail and result in garbled data. Fortunately, each scheme
 * has techniques to manage noise.
 *
 * BFV and CKKS are leveled schemes. This means that if the number of homomorphic operations is known in advance,
 * one can choose parameters for the scheme to accomodate the predicted noise. Generally speaking,
 * the maximum number of multiplications required to produce any output of a circuit determines the
 * scheme parameters. For those with a digital logic background, this is analogous to worst-case propagation
 * delay determining the clock cycle of the circuit. Choosing parameters that accomodate more noise unfortunately
 * results in more time spent on each operation.
 *
 * BFV and CKKS support SIMD, also known as batching. This means that a single addition or multiplication
 * operation can be applied to many numbers at once.
 *
 * While TFHE is also theoretically a leveled scheme, it primarily uses bootstrapping as the method to manage noise.
 * Under bootstrapping, one homomorphically decrypts a ciphertext, which resets the noise level. With TFHE,
 * this is a fast procedure. Bootstrapping can be applied repeatedly, resulting in an arbitrary number of computations.
 *
 * TFHE is similar to digital logic design; one expresses boolean circuits of and, or, not, etc. gates
 * on bits. While most flexible, it tends to have larger ciphertext expansion and be slower at numeric
 * computation than BFV and CKKS.
 *
 * CKKS and BFV can also perform bootstrapping, but it is generally expensive and not built into the underlying
 * implementation Sunscreen provides.
 */
pub enum SchemeType {
    /**
     * [BFV](https://eprint.iacr.org/2012/144.pdf) is a leveled scheme on polynomials in a cyclotomic
     * ring. The coefficients of a plaintext form a 2x(N/2) matrix (where N is the polynomial degree).
     * Sunscreen automatically chooses the polynomial degree depending on the circuit. Each coefficient is
     * an integer mod p (p is a scheme parameter and is the plaintext modulus). One can encode several different
     * meanings onto these coefficients:
     *
     * * An integer x modulo p by setting the x^0 term to x and the remaining terms to 0 (i.e. scalar encoder).
     * This encoding requires p be the desired maximum representable value. Overflow causes wrapping as
     * one would expect. This encoding is generally inefficient.
     * * An integer x decomposed into digits, where each digit is a coefficient in the plaintext polynomial.
     * One may represent numbers larger than p with this technique. P should be chosen to accomodate the number
     * of operations one wishes to perform so that no digit overflows under addition and multiplication. Overflow
     * causes weird answers. Since this encoding typically allows for a smaller plaintext modulo, Sunscreen
     * can choose parameters that result in low latency.
     * * A 2x(N/2) SIMD vector of integers modulo p. Overflow wraps lane-wise, as expected. This encoding
     * generally maximizes throughput when calulating many numbers. While the representation forms a matrix,
     * multiplication and addition both execute element-wise; multiplication is *not* defined as matrix multiplation.
     * This SIMD computation is also referred to on the literature as batching.
     *
     * Each of these encoding schemes supports both signed and unsigned values.
     *
     * Under BFV, each homomorphic operation introduces noise, with ciphertext-ciphertext multiplication
     * creating the most by a quadratic margin. Additionally, multiplication is the slowest operation. To
     * reduce noise under repeated multiplications, Sunscreen will automatically insert relinearization operations.
     *
     * After some number of operations (parameter-dependent), ciphertexts contain too much noise and
     * decryption results in garbled data. Sunscreen automatically chooses the parameters to accomodate
     * the noise growth in a given circuit at the expense of execution speed.
     *
     * One can think of parameters as a tradeoff between accomodating more noise and faster execution. For a given security
     * level, there are several possible parameter sets. These sets are ordered from accomodating the smallest
     * level of noise to largest. Moving from one set to the next results in every operation requiring ~4x the
     * runtime, but also results in 2x the SIMD lanes. Thus, when using SIMD plaintexts, the amortized
     * throughput resulting from using the next parameter set is 2x lower than the previous. The smallest 2
     * parameter sets fail to even generate relinearization keys and fail to even perform a single multiplication
     * when using batching, while the largest can perform over 25 multiplications with batching.
     *
     * When using SIMD, Sunscreen supports rotating column SIMD lanes left and right and switching the rows
     * of the matrix.
     *
     * Pros:
     * * Most efficient way to do integer artithmetic.
     * * Exact values.
     * * Good ciphertext expansion when using batching.
     * * Galois keys (needed if circuit does rotations or row swapping) can be compactly generated.
     * * Relinearization keys (needed if circuit does multiplications) can be compactly generated.
     *
     * Cons:
     * * Bootstrapping not natively supported and isn't fast if one does implement it.
     * * Some operations (e.g. comparison, division) are not easy to implement and any implementation
     * will be approximate and/or particular to the scheme parameters.
     */
    Bfv,

    /**
     * Note, this scheme is not yet implemented in Sunscreen.
     *
     * [CKKS](https://eprint.iacr.org/2016/421.pdf) is a leveled scheme in polynomials in a cyclotomic
     * ring. The coeffiecients of a plaintext form a vector of N/2 elements (where N is the polynomial degree),
     * where each element is a complex integer mod c (c is the ciphertext modulus for the current rescale level).
     *
     * Homomorphic operantions in CKKS include addition and multiplication of ciphertexts. That is,
     * one can homomorphically add and multiply floating-point-esque numbers.
     *
     * As one performs operations, noise manifests as increasing loss of precision, similar to floating-point
     * round-off, but more pronounced. Rescaling operations (automatically inserted by Sunscreen) reduce
     * future noise growth.
     *
     * Sunscreen uses Microsoft SEAL for the backing CKKS implementation.
     *
     * Pros:
     * * Most efficient way to do floating-point arithmetic.
     * * Natively supports complex arithmetic and complex conjugation.
     * * Good ciphertext expansion when using batching.
     * * Galois keys (needed if circuit does rotations or row swapping) can be compactly generated.
     *
     * Cons:
     * * Bootstrapping not natively supported and isn't fast if one does implement it.
     * * Some operations (e.g. comparison) are not easy to implement and any implementation
     * will be particular to the scheme parameters.
     * * Not suitable when exact values are needed.
     */
    Ckks,

    /**
     * Note, scheme not yet implemented in Sunscreen.
     *
     * The [TFHE](https://tfhe.github.io/tfhe/) scheme. While this scheme theoretically supports leveled computations, bootstrapping
     * is very fast in TFHE so implementors rarely support leveling. This scheme supports homomorphic binary
     * gate operations as well as an intrinsic multiplexer opeeration.
     *
     * This scheme is general enough to implement a [CPU](https://www.usenix.org/system/files/sec21summer_matsuoka.pdf)
     * and homomorphically run any C program you like. The linked processor runs at 1.3Hz on a
     * 96-core, 4GPU AWS instance that costs $8/hr. Additionally, you don't know when the computer
     * halts so you have to run for an upper bound number of cycles and hope your computation completes,
     * so this doesn't magically defeat some of FHE's challenges.
     *
     * Pros:
     * * Ultra fast (~10us) Bootstrapping allows for unlimited computation.
     * * Binary representation is completely flexible.
     * * Exact values.
     *
     * Cons:
     * * No batching results in poor throughput for complex operations.
     * * Arithmetic is significantly slower than BFV and CKKS schemes.
     * * Without a good library of emergent types, binary is very unwieldy. Must carry-select
     * adders, shifts, etc. to accomplish arithmetic.
     * * Large ciphertext expansion; a single bit of plaintext requires 10,000 bits of ciphertext.
     * * Required bootstrapping key is ~16MB.
     */
    Tfhe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/**
 * Contains information about a node in the circuit graph.
 */
pub struct NodeInfo {
    /**
     * The operation this node represents.
     */
    pub operation: Operation,
}

impl ToString for NodeInfo {
    fn to_string(&self) -> String {
        format!("{:#?}", self.operation)
    }
}

impl NodeInfo {
    fn new(operation: Operation) -> Self {
        Self { operation }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
/**
 * Contains information about an edge between nodes in the circuit graph.
 */
pub enum EdgeInfo {
    /**
     * The source node is the left input to a binary operation.
     */
    LeftOperand,

    /**
     * The source node is the right input to fa binary operation.
     */
    RightOperand,

    /**
     * The source node is the single input to a unary operation.
     */
    UnaryOperand,
}

type IRGraph = StableGraph<NodeInfo, EdgeInfo>;

#[derive(Debug, Clone, Serialize, Deserialize)]
/**
 * The intermediate representation for an FHE circuit used in the compiler back-end.
 *
 * Other modules may transform these using the [forward_traverse](`Self::forward_traverse`)
 * and [reverse_traverse](`Self::reverse_traverse`) methods, or iterate over the graph
 * member for analysis or execution.
 *
 * The graph construction methods `append_*` take NodeIndex types as arguments. These
 * indices must refer to other nodes in the graph.
 */
pub struct Circuit {
    /**
     * The scheme type this circuit will run under.
     */
    pub scheme: SchemeType,

    /**
     * The underlying dependency graph.
     */
    pub graph: IRGraph,
}

impl PartialEq for Circuit {
    fn eq(&self, b: &Self) -> bool {
        is_isomorphic_matching(
            &Graph::from(self.graph.clone()),
            &Graph::from(b.graph.clone()),
            |n1, n2| n1 == n2,
            |e1, e2| e1 == e2,
        )
    }
}

impl Circuit {
    /**
     * Create a new new empty intermediate representation.
     */
    pub fn new(scheme: SchemeType) -> Self {
        Self {
            scheme,
            graph: StableGraph::new(),
        }
    }

    fn append_2_input_node(
        &mut self,
        operation: Operation,
        x: NodeIndex,
        y: NodeIndex,
    ) -> NodeIndex {
        let new_node = self.graph.add_node(NodeInfo::new(operation));

        self.graph.update_edge(x, new_node, EdgeInfo::LeftOperand);
        self.graph.update_edge(y, new_node, EdgeInfo::RightOperand);

        new_node
    }

    fn append_1_input_node(&mut self, operation: Operation, x: NodeIndex) -> NodeIndex {
        let new_node = self.graph.add_node(NodeInfo::new(operation));

        self.graph.update_edge(x, new_node, EdgeInfo::UnaryOperand);

        new_node
    }

    fn append_0_input_node(&mut self, operation: Operation) -> NodeIndex {
        let new_node = self.graph.add_node(NodeInfo::new(operation));

        new_node
    }

    /**
     * Appends a negate operation that depends on operand `x`.
     */
    pub fn append_negate(&mut self, x: NodeIndex) -> NodeIndex {
        self.append_1_input_node(Operation::Negate, x)
    }

    /**
     * Appends a multiply operation that depends on the operands `x` and `y`.
     */
    pub fn append_multiply(&mut self, x: NodeIndex, y: NodeIndex) -> NodeIndex {
        self.append_2_input_node(Operation::Multiply, x, y)
    }

    /**
     * Appends an add operation that depends on the operands `x` and `y`.
     */
    pub fn append_add(&mut self, x: NodeIndex, y: NodeIndex) -> NodeIndex {
        self.append_2_input_node(Operation::Add, x, y)
    }

    /**
     * Appends a subtract operation that depends on the operands `x` and `y`.
     */
    pub fn append_sub(&mut self, x: NodeIndex, y: NodeIndex) -> NodeIndex {
        self.append_2_input_node(Operation::Sub, x, y)
    }

    /**
     * Appends an input ciphertext with the given name.
     */
    pub fn append_input_ciphertext(&mut self, id: usize) -> NodeIndex {
        self.append_0_input_node(Operation::InputCiphertext(id))
    }

    /**
     * Appends a constant literal unencrypted.
     *
     * * `value`: The integer or floating-point value in the literal.
     */
    pub fn append_input_literal(&mut self, value: OuterLiteral) -> NodeIndex {
        self.append_0_input_node(Operation::Literal(value))
    }

    /**
     * Sppends a node designating `x` as an output of the circuit.
     */
    pub fn append_output_ciphertext(&mut self, x: NodeIndex) -> NodeIndex {
        self.append_1_input_node(Operation::OutputCiphertext, x)
    }

    /**
     * Appends an operation that relinearizes `x`.
     */
    pub fn append_relinearize(&mut self, x: NodeIndex) -> NodeIndex {
        self.append_1_input_node(Operation::Relinearize, x)
    }

    /**
     * Appends an operation that rotates ciphertext `x` left by the literal node at `y` places.
     *
     * # Remarks
     * Recall that BFV has 2 rows in a SIMD vector. This rotates each row.
     * CKKS has one large vector.
     */
    pub fn append_rotate_left(&mut self, x: NodeIndex, y: NodeIndex) -> NodeIndex {
        self.append_2_input_node(Operation::ShiftLeft, x, y)
    }

    /**
     * Appends an operation that rotates ciphertext `x` right by the literal node at `y` places.
     *      
     * # Remarks
     * Recall that BFV has 2 rows in a SIMD vector. This rotates each row.
     * CKKS has one large vector.
     */
    pub fn append_rotate_right(&mut self, x: NodeIndex, y: NodeIndex) -> NodeIndex {
        self.append_2_input_node(Operation::ShiftRight, x, y)
    }

    /**
     * A specialized topological DAG traversal that allows the following graph
     * mutations during traversal:
     * * Delete the current node
     * * Insert nodoes after current node
     * * Add new nodes with no dependencies
     *
     * Any other graph mutation will likely result in unvisited nodes.
     *
     * * `callback`: A closure that receives the current node index and an object allowing
     *   you to make graph queryes. This closure returns a transform list.
     *   [`forward_traverse`](Self::forward_traverse) will apply these transformations
     *   before continuing the traversal.
     */
    pub fn forward_traverse<F>(&mut self, callback: F)
    where
        F: FnMut(GraphQuery, NodeIndex) -> TransformList,
    {
        self.traverse(true, callback);
    }

    /**
     * A specialized reverse topological DAG traversal that allows the following graph
     * mutations during traversal:
     * * Delete the current node
     * * Insert nodoes after current node
     * * Add new nodes with no dependencies
     *
     * Any other graph mutation will likely result in unvisited nodes.
     *
     * * `callback`: A closure that receives the current node index and an object allowing
     *   you to make graph queryes. This closure returns a transform list.
     *   [`reverse_traverse`](Self::reverse_traverse) will apply these transformations
     *   before continuing the traversal.
     */
    pub fn reverse_traverse<F>(&mut self, callback: F)
    where
        F: FnMut(GraphQuery, NodeIndex) -> TransformList,
    {
        self.traverse(false, callback);
    }

    /**
     * Remove the given node.
     */
    pub fn remove_node(&mut self, id: NodeIndex) {
        self.graph.remove_node(id);
    }

    fn traverse<F>(&mut self, forward: bool, mut callback: F)
    where
        F: FnMut(GraphQuery, NodeIndex) -> TransformList,
    {
        let mut ready: HashSet<NodeIndex> = HashSet::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let prev_direction = if forward {
            Direction::Incoming
        } else {
            Direction::Outgoing
        };
        let next_direction = if forward {
            Direction::Outgoing
        } else {
            Direction::Incoming
        };

        let mut ready_nodes: Vec<NodeIndex> = self
            .graph
            .node_identifiers()
            .filter(|&x| {
                self.graph
                    .neighbors_directed(x, prev_direction)
                    .next()
                    .is_none()
            })
            .collect();

        for i in &ready_nodes {
            ready.insert(*i);
        }

        while let Some(n) = ready_nodes.pop() {
            visited.insert(n);

            // Remember the next nodes from the current node in case it gets deletes.
            let next_nodes: Vec<NodeIndex> =
                self.graph.neighbors_directed(n, next_direction).collect();

            let mut transforms = callback(GraphQuery(self), n);

            // Apply the transforms the callback produced
            transforms.apply(self);

            let node_ready = |n: NodeIndex| {
                self.graph
                    .neighbors_directed(n, prev_direction)
                    .all(|m| visited.contains(&m))
            };

            // If the node still exists, push all its ready dependents
            if self.graph.contains_node(n) {
                for i in self.graph.neighbors_directed(n, next_direction) {
                    if !ready.contains(&i) && node_ready(i) {
                        ready.insert(i);
                        ready_nodes.push(i);
                    }
                }
            }

            // Iterate through the next nodes that existed before visitin this node.
            for i in next_nodes {
                if !ready.contains(&i) && node_ready(i) {
                    ready.insert(i);
                    ready_nodes.push(i);
                }
            }

            // Iterate through any sources/sinks the callback may have added.
            let sources = self.graph.node_identifiers().filter(|&x| {
                self.graph
                    .neighbors_directed(x, prev_direction)
                    .next()
                    .is_none()
            });

            for i in sources {
                if !ready.contains(&i) {
                    ready.insert(i);
                    ready_nodes.push(i);
                }
            }
        }
    }

    /**
     * Returns the node indices of output ciphertexts
     */
    pub fn get_outputs(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph
            .node_indices()
            .filter(|g| match self.graph[*g].operation {
                Operation::OutputCiphertext => true,
                _ => false,
            })
    }

    /**
     * Runs tree shaking and returns a derived Circuit with only
     * dependencies required to run the requested nodes.
     *
     * * `nodes`: indices specifying a set of nodes in the graph. Prune return a new
     *   [`Circuit`] containing nodes in the transitive closure
     *   of this set.
     */
    pub fn prune(&self, nodes: &[NodeIndex]) -> Circuit {
        let mut compact_graph = Graph::from(self.graph.clone());
        compact_graph.reverse();

        let topo = toposort(&compact_graph, None).unwrap();
        let (res, revmap) = dag_to_toposorted_adjacency_list(&compact_graph, &topo);
        let (_, closure) = dag_transitive_reduction_closure(&res);

        let mut closure_set = HashSet::new();

        let mut visit: Vec<NodeIndex> = vec![];

        for n in nodes {
            let mapped_id = revmap[n.index()];
            visit.push(mapped_id);
            closure_set.insert(mapped_id);
        }

        while visit.len() > 0 {
            let node = visit.pop().expect("Fatal error: prune queue was empty.");

            for edge in closure.neighbors(node) {
                if !closure_set.contains(&edge) {
                    closure_set.insert(edge);
                    visit.push(edge);
                }
            }
        }

        compact_graph.reverse();

        let pruned = compact_graph.filter_map(
            |id, n| {
                if closure_set.contains(&revmap[id.index()]) {
                    Some(n.clone())
                } else {
                    None
                }
            },
            |_, e| Some(e.clone()),
        );

        Self {
            scheme: self.scheme,
            graph: StableGraph::from(pruned),
        }
    }

    /**
     * Validates this [`Circuit`] for correctness.
     */
    pub fn validate(&self) -> Result<()> {
        let errors = validation::validate_ir(self);

        if errors.len() > 0 {
            return Err(Error::IRError(errors));
        }

        Ok(())
    }
}

/**
 * A wrapper for ascertaining the structure of the underlying [`Circuit`].
 * This type is used in [`Circuit::forward_traverse`] and
 * [`Circuit::reverse_traverse`] callbacks.
 */
pub struct GraphQuery<'a>(&'a Circuit);

impl<'a> GraphQuery<'a> {
    /**
     * Creates a new [`GraphQuery`] from a reference to an [`Circuit`].
     */
    pub fn new(ir: &'a Circuit) -> Self {
        Self(ir)
    }

    /**
     * Returns the [`NodeInfo`] for the graph node with the given index `x`.
     */
    pub fn get_node(&self, x: NodeIndex) -> &NodeInfo {
        &self.0.graph[x]
    }

    /**
     * Returns the children or parents of the node with index `x`.` If direction is
     * [`Direction::Outgoing`], this will return the children. If the direction is
     * [`Direction::Incoming`], this will return the parents.
     *
     * Typically, you want children writing forward traversal compiler passes and
     * parents when writing reverse traversal compiler passes.
     */
    pub fn neighbors_directed(&self, x: NodeIndex, direction: Direction) -> Neighbors<EdgeInfo> {
        self.0.graph.neighbors_directed(x, direction)
    }

    /**
     * Returns incoming our outgoing edges for the node with index `x`.`
     *
     * Typically, you want outgoing writing forward traversal compiler passes and
     * incoming when writing reverse traversal compiler passes.
     */
    pub fn edges_directed(&self, x: NodeIndex, direction: Direction) -> Edges<EdgeInfo, Directed> {
        self.0.graph.edges_directed(x, direction)
    }
}

#[derive(Debug, Clone)]
/**
 * A transform for an [`Circuit`]. Callbacks in
 * [`Circuit::forward_traverse`] and
 * [`Circuit::reverse_traverse`] should emit these to update the
 * graph.
 *
 * Each of these variants use a [`TransformNodeIndex`] to reference either a node that
 * currently exists in the graph (i.e. [`TransformNodeIndex::NodeIndex`]), or a node that
 * will result from a previous transform in the [`TransformList`]. I.e. [`TransformNodeIndex::DeferredIndex`]
 */
pub enum IRTransform {
    /**
     * Appends an add node.
     */
    AppendAdd(TransformNodeIndex, TransformNodeIndex),

    /**
     * Appends a multiply node.
     */
    AppendMultiply(TransformNodeIndex, TransformNodeIndex),

    /**
     * Appends an input ciphertext
     */
    AppendInputCiphertext(usize),

    /**
     * Appends an output ciphertext node.
     */
    AppendOutputCiphertext(TransformNodeIndex),

    /**
     * Appends a relinearize node.
     */
    AppendRelinearize(TransformNodeIndex),

    /**
     * Appends a subtract node.
     */
    AppendSub(TransformNodeIndex, TransformNodeIndex),

    /**
     * Removes a node.
     */
    RemoveNode(TransformNodeIndex),

    /**
     * Appends a negate node.
     */
    AppendNegate(TransformNodeIndex),

    /**
     * Remove a graph edge between two nodes..
     */
    RemoveEdge(TransformNodeIndex, TransformNodeIndex),

    /**
     * Add a graph edge between two nodes.
     */
    AddEdge(TransformNodeIndex, TransformNodeIndex, EdgeInfo),
}

/**
 * Transforms can refer to nodes that already exist in the graph or nodes that don't
 * yet exist in the graph, but will be inserted in a previous transform.
 */
#[derive(Debug, Clone, Copy)]
pub enum TransformNodeIndex {
    /**
     * This node index refers to a pre-existing node in the graph.
     */
    NodeIndex(NodeIndex),

    /**
     * This node index refers to a node in the [`TransformList`] that has not yet been
     * added to the graph.
     */
    DeferredIndex(DeferredIndex),
}

/**
 * The index type of a node that exists in a transform list, but does not yet exist in
 * the intermediate representation graph.
 */
pub type DeferredIndex = usize;

impl Into<TransformNodeIndex> for DeferredIndex {
    fn into(self) -> TransformNodeIndex {
        TransformNodeIndex::DeferredIndex(self)
    }
}

impl Into<TransformNodeIndex> for NodeIndex {
    fn into(self) -> TransformNodeIndex {
        TransformNodeIndex::NodeIndex(self)
    }
}

/**
 * A list of tranformations to be applied to the [`Circuit`] graph.
 */
pub struct TransformList {
    transforms: Vec<IRTransform>,
    inserted_node_ids: Vec<Option<NodeIndex>>,
}

impl Default for TransformList {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformList {
    /**
     * Creates an empty transform list.
     */
    pub fn new() -> Self {
        Self {
            transforms: vec![],
            inserted_node_ids: vec![],
        }
    }

    /**
     * Pushes a transform into the list and returns the index of the pushed transform
     * suitable for use in [`TransformNodeIndex::DeferredIndex`].
     */
    pub fn push(&mut self, transform: IRTransform) -> DeferredIndex {
        self.transforms.push(transform);

        self.transforms.len() - 1
    }

    /**
     * Applies every transform in the list to the given graph. Resoves any deferred
     * indices after placing nodes in the graph.
     *
     * # Panics
     * If any deferred index is out of bounds or refers to a previous operation that didn't
     * result in a node being added, this function will panic. For example, if an [`IRTransform::AppendAdd`]
     * refers to the index of a [`IRTransform::RemoveEdge`] transform, a panic will result.
     */
    pub fn apply(&mut self, ir: &mut Circuit) {
        for t in self.transforms.clone().iter() {
            let inserted_node_id = match t {
                AppendAdd(x, y) => {
                    self.apply_2_input(ir, *x, *y, |ir, x, y| Some(ir.append_add(x, y)))
                }
                AppendMultiply(x, y) => {
                    self.apply_2_input(ir, *x, *y, |ir, x, y| Some(ir.append_multiply(x, y)))
                }
                AppendInputCiphertext(id) => Some(ir.append_input_ciphertext(*id)),
                AppendOutputCiphertext(x) => {
                    self.apply_1_input(ir, *x, |ir, x| Some(ir.append_output_ciphertext(x)))
                }
                AppendRelinearize(x) => {
                    self.apply_1_input(ir, *x, |ir, x| Some(ir.append_relinearize(x)))
                }
                AppendSub(x, y) => {
                    self.apply_2_input(ir, *x, *y, |ir, x, y| Some(ir.append_sub(x, y)))
                }
                RemoveNode(x) => {
                    let x = self.materialize_index(*x);

                    ir.remove_node(x);

                    None
                }
                AppendNegate(x) => self.apply_1_input(ir, *x, |ir, x| Some(ir.append_negate(x))),
                RemoveEdge(x, y) => {
                    let x = self.materialize_index(*x);
                    let y = self.materialize_index(*y);

                    ir.graph.remove_edge(
                        ir.graph
                            .find_edge(x, y)
                            .expect("Fatal error: attempted to remove nonexistent edge."),
                    );

                    None
                }
                AddEdge(x, y, edge_info) => {
                    let x = self.materialize_index(*x);
                    let y = self.materialize_index(*y);

                    ir.graph.update_edge(x, y, *edge_info);

                    None
                }
            };

            self.inserted_node_ids.push(inserted_node_id);
        }
    }

    fn apply_1_input<F>(
        &mut self,
        ir: &mut Circuit,
        x: TransformNodeIndex,
        callback: F,
    ) -> Option<NodeIndex>
    where
        F: FnOnce(&mut Circuit, NodeIndex) -> Option<NodeIndex>,
    {
        let x = self.materialize_index(x);

        callback(ir, x)
    }

    fn apply_2_input<F>(
        &mut self,
        ir: &mut Circuit,
        x: TransformNodeIndex,
        y: TransformNodeIndex,
        callback: F,
    ) -> Option<NodeIndex>
    where
        F: FnOnce(&mut Circuit, NodeIndex, NodeIndex) -> Option<NodeIndex>,
    {
        let x = self.materialize_index(x);
        let y = self.materialize_index(y);

        callback(ir, x, y)
    }

    fn materialize_index(&self, x: TransformNodeIndex) -> NodeIndex {
        match x {
            NodeIndex(x) => x,
            DeferredIndex(x) => self.inserted_node_ids[x]
                .expect(&format!("Fatal error: No such deferred node index :{}", x)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_simple_dag() -> Circuit {
        let mut ir = Circuit::new(SchemeType::Bfv);

        let ct = ir.append_input_ciphertext(0);
        let l1 = ir.append_input_literal(OuterLiteral::from(7i64));
        let add = ir.append_add(ct, l1);
        let l2 = ir.append_input_literal(OuterLiteral::from(5u64));
        ir.append_multiply(add, l2);

        ir
    }

    #[test]
    fn can_build_simple_dag() {
        let ir = create_simple_dag();

        assert_eq!(ir.graph.node_count(), 5);

        let nodes = ir
            .graph
            .node_identifiers()
            .map(|i| (i, &ir.graph[i]))
            .collect::<Vec<(NodeIndex, &NodeInfo)>>();

        assert_eq!(nodes[0].1.operation, Operation::InputCiphertext(0));
        assert_eq!(
            nodes[1].1.operation,
            Operation::Literal(OuterLiteral::from(7i64))
        );
        assert_eq!(nodes[2].1.operation, Operation::Add);
        assert_eq!(
            nodes[3].1.operation,
            Operation::Literal(OuterLiteral::from(5u64))
        );
        assert_eq!(nodes[4].1.operation, Operation::Multiply);

        assert_eq!(
            ir.graph
                .neighbors_directed(nodes[0].0, Direction::Outgoing)
                .next()
                .unwrap(),
            nodes[2].0
        );
        assert_eq!(
            ir.graph
                .neighbors_directed(nodes[1].0, Direction::Outgoing)
                .next()
                .unwrap(),
            nodes[2].0
        );
        assert_eq!(
            ir.graph
                .neighbors_directed(nodes[2].0, Direction::Outgoing)
                .next()
                .unwrap(),
            nodes[4].0
        );
        assert_eq!(
            ir.graph
                .neighbors_directed(nodes[3].0, Direction::Outgoing)
                .next()
                .unwrap(),
            nodes[4].0
        );
        assert_eq!(
            ir.graph
                .neighbors_directed(nodes[4].0, Direction::Outgoing)
                .next(),
            None
        );
    }

    #[test]
    fn can_forward_traverse() {
        let mut ir = create_simple_dag();

        let mut visited = vec![];

        ir.forward_traverse(|_, n| {
            visited.push(n);
            TransformList::default()
        });

        assert_eq!(
            visited,
            vec![
                NodeIndex::from(3),
                NodeIndex::from(1),
                NodeIndex::from(0),
                NodeIndex::from(2),
                NodeIndex::from(4)
            ]
        );
    }

    #[test]
    fn can_reverse_traverse() {
        let mut ir = create_simple_dag();

        let mut visited = vec![];

        ir.reverse_traverse(|_, n| {
            visited.push(n);
            TransformList::default()
        });

        assert_eq!(
            visited,
            vec![
                NodeIndex::from(4),
                NodeIndex::from(2),
                NodeIndex::from(0),
                NodeIndex::from(1),
                NodeIndex::from(3)
            ]
        );
    }

    #[test]
    fn can_delete_during_traversal() {
        let mut ir = create_simple_dag();

        let mut visited = vec![];

        ir.reverse_traverse(|_, n| {
            visited.push(n);
            // Delete the addition
            if n.index() == 2 {
                let mut transforms = TransformList::new();
                transforms.push(RemoveNode(n.into()));

                transforms
            } else {
                TransformList::default()
            }
        });

        assert_eq!(
            visited,
            vec![
                NodeIndex::from(4),
                NodeIndex::from(2),
                NodeIndex::from(0),
                NodeIndex::from(1),
                NodeIndex::from(3)
            ]
        );
    }

    #[test]
    fn can_append_during_traversal() {
        let mut ir = create_simple_dag();

        let mut visited = vec![];

        ir.forward_traverse(|_, n| {
            visited.push(n);

            // Delete the addition
            if n.index() == 2 {
                let mut transforms = TransformList::new();
                transforms.push(AppendMultiply(n.into(), NodeIndex::from(1).into()));

                transforms
            } else {
                TransformList::default()
            }
        });

        assert_eq!(
            visited,
            vec![
                NodeIndex::from(3),
                NodeIndex::from(1),
                NodeIndex::from(0),
                NodeIndex::from(2),
                NodeIndex::from(4),
                NodeIndex::from(5),
            ]
        );
    }

    #[test]
    fn can_prune_ir() {
        let mut ir = Circuit::new(SchemeType::Bfv);

        let ct = ir.append_input_ciphertext(0);
        let l1 = ir.append_input_literal(OuterLiteral::from(7i64));
        let add = ir.append_add(ct, l1);
        let l2 = ir.append_input_literal(OuterLiteral::from(5u64));
        ir.append_multiply(add, l2);

        let pruned = ir.prune(&vec![add]);

        let mut expected_ir = Circuit::new(SchemeType::Bfv);
        let ct = expected_ir.append_input_ciphertext(0);
        let l1 = expected_ir.append_input_literal(OuterLiteral::from(7i64));
        expected_ir.append_add(ct, l1);

        assert_eq!(pruned, expected_ir);
    }

    #[test]
    fn can_prune_graph_with_removed_nodes() {
        let mut ir = Circuit::new(SchemeType::Bfv);

        let ct = ir.append_input_ciphertext(0);
        let rem = ir.append_input_ciphertext(1);
        ir.remove_node(rem);
        let l1 = ir.append_input_literal(OuterLiteral::from(7i64));
        let rem = ir.append_input_ciphertext(1);
        ir.remove_node(rem);
        let add = ir.append_add(ct, l1);
        let rem = ir.append_input_ciphertext(1);
        ir.remove_node(rem);
        let l2 = ir.append_input_literal(OuterLiteral::from(5u64));
        ir.append_multiply(add, l2);
        let rem = ir.append_input_ciphertext(1);
        ir.remove_node(rem);

        let pruned = ir.prune(&vec![add]);

        let mut expected_ir = Circuit::new(SchemeType::Bfv);
        let ct = expected_ir.append_input_ciphertext(0);
        let l1 = expected_ir.append_input_literal(OuterLiteral::from(7i64));
        expected_ir.append_add(ct, l1);

        assert_eq!(pruned, expected_ir);
    }

    #[test]
    fn can_prune_with_multiple_nodes() {
        let mut ir = Circuit::new(SchemeType::Bfv);

        let ct1 = ir.append_input_ciphertext(0);
        let ct2 = ir.append_input_ciphertext(1);
        let ct3 = ir.append_input_ciphertext(2);
        let neg1 = ir.append_negate(ct1);
        let neg2 = ir.append_negate(ct2);
        let neg3 = ir.append_negate(ct3);
        let o1 = ir.append_output_ciphertext(neg1);
        ir.append_output_ciphertext(neg2);
        ir.append_output_ciphertext(neg3);

        let pruned = ir.prune(&vec![o1, neg2]);

        let mut expected_ir = Circuit::new(SchemeType::Bfv);
        let ct1 = expected_ir.append_input_ciphertext(0);
        let ct2 = expected_ir.append_input_ciphertext(1);
        let neg1 = expected_ir.append_negate(ct1);
        expected_ir.append_negate(ct2);
        expected_ir.append_output_ciphertext(neg1);

        assert_eq!(pruned, expected_ir);
    }

    #[test]
    fn pruning_empty_node_list_results_in_empty_graph() {
        let mut ir = Circuit::new(SchemeType::Bfv);

        let ct1 = ir.append_input_ciphertext(0);
        let ct2 = ir.append_input_ciphertext(1);
        let ct3 = ir.append_input_ciphertext(2);
        let neg1 = ir.append_negate(ct1);
        let neg2 = ir.append_negate(ct2);
        let neg3 = ir.append_negate(ct3);
        ir.append_output_ciphertext(neg1);
        ir.append_output_ciphertext(neg2);
        ir.append_output_ciphertext(neg3);

        let pruned = ir.prune(&vec![]);

        let expected_ir = Circuit::new(SchemeType::Bfv);

        assert_eq!(pruned, expected_ir);
    }
}