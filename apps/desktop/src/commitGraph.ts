/**
 * Lane layout for a commit graph drawn one row per commit, newest first
 * (topological order). Each row says which lanes pass through it, which lanes
 * end at its commit, and which lanes leave it toward its parents.
 */

export type GraphCommit = { id: string; parents: readonly string[] };

export type GraphRow = {
  /** Lane of the commit's node. */
  column: number;
  /** Lanes that continue straight through the row. */
  through: number[];
  /** Lanes that come from above into the node (children of this commit). */
  incoming: number[];
  /** Lanes that leave the node downward, one per parent. */
  outgoing: number[];
  /** Number of lanes in use in this row, for its width. */
  width: number;
};

function freeLane(lanes: (string | null)[]): number {
  const index = lanes.indexOf(null);
  if (index >= 0) return index;
  lanes.push(null);
  return lanes.length - 1;
}

export function layoutGraph(commits: readonly GraphCommit[]): GraphRow[] {
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  for (const commit of commits) {
    const incoming = lanes.flatMap((expected, index) => (expected === commit.id ? [index] : []));
    const column = incoming[0] ?? freeLane(lanes);
    const through = lanes.flatMap((expected, index) => (expected !== null && expected !== commit.id ? [index] : []));
    for (const index of incoming) lanes[index] = null;

    const outgoing: number[] = [];
    commit.parents.forEach((parent, position) => {
      const existing = lanes.indexOf(parent);
      if (existing >= 0) {
        outgoing.push(existing);
        return;
      }
      const lane = position === 0 && lanes[column] === null ? column : freeLane(lanes);
      lanes[lane] = parent;
      outgoing.push(lane);
    });
    const width = Math.max(column + 1, ...incoming.map((lane) => lane + 1), ...through.map((lane) => lane + 1), ...outgoing.map((lane) => lane + 1));
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    rows.push({ column, through, incoming, outgoing, width });
  }
  return rows;
}
