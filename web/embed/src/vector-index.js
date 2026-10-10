// Brute-force cosine index over one contiguous Float32Array.
// Vectors are L2-normalised on insert, so search is a plain dot product.

function normaliseInto(dst, offset, src, dimensions) {
  let sum = 0;
  for (let i = 0; i < dimensions; i++) sum += src[i] * src[i];
  const scale = sum > 0 ? 1 / Math.sqrt(sum) : 0;
  for (let i = 0; i < dimensions; i++) dst[offset + i] = src[i] * scale;
}

export class VectorIndex {
  #dimensions;
  #data;
  #ids = [];           // slot -> id
  #slots = new Map();  // id -> slot
  #query;

  constructor(dimensions, { initialCapacity = 1024 } = {}) {
    if (!Number.isInteger(dimensions) || dimensions <= 0) {
      throw new RangeError("VectorIndex: dimensions must be a positive integer");
    }
    this.#dimensions = dimensions;
    this.#data = new Float32Array(Math.max(1, initialCapacity) * dimensions);
    this.#query = new Float32Array(dimensions);
  }

  get dimensions() {
    return this.#dimensions;
  }

  get size() {
    return this.#ids.length;
  }

  has(id) {
    return this.#slots.has(id);
  }

  // Adding an id that already exists replaces its vector.
  add(id, vector) {
    this.#check(vector);
    let slot = this.#slots.get(id);
    if (slot === undefined) {
      slot = this.#ids.length;
      if ((slot + 1) * this.#dimensions > this.#data.length) {
        const grown = new Float32Array(this.#data.length * 2);
        grown.set(this.#data);
        this.#data = grown;
      }
      this.#ids.push(id);
      this.#slots.set(id, slot);
    }
    normaliseInto(this.#data, slot * this.#dimensions, vector, this.#dimensions);
  }

  // Returns true if the id was present. The last vector is moved into the hole.
  remove(id) {
    const slot = this.#slots.get(id);
    if (slot === undefined) return false;
    const d = this.#dimensions;
    const last = this.#ids.length - 1;
    if (slot !== last) {
      this.#data.copyWithin(slot * d, last * d, (last + 1) * d);
      const moved = this.#ids[last];
      this.#ids[slot] = moved;
      this.#slots.set(moved, slot);
    }
    this.#ids.pop();
    this.#slots.delete(id);
    return true;
  }

  clear() {
    this.#ids.length = 0;
    this.#slots.clear();
  }

  // [{ id, score }] by cosine similarity, best first.
  search(vector, k = 10) {
    this.#check(vector);
    const d = this.#dimensions;
    const n = this.#ids.length;
    k = Math.min(Math.floor(k), n);
    if (!(k > 0)) return [];

    const q = this.#query;
    normaliseInto(q, 0, vector, d);
    const data = this.#data;

    // Sorted top-k buffer (ascending by score, worst at index 0).
    const scores = new Float32Array(k).fill(-Infinity);
    const slots = new Int32Array(k).fill(-1);
    for (let s = 0, base = 0; s < n; s++, base += d) {
      let dot = 0;
      for (let i = 0; i < d; i++) dot += q[i] * data[base + i];
      if (dot <= scores[0]) continue;
      let j = 0;
      while (j + 1 < k && scores[j + 1] < dot) {
        scores[j] = scores[j + 1];
        slots[j] = slots[j + 1];
        j++;
      }
      scores[j] = dot;
      slots[j] = s;
    }

    const out = [];
    for (let j = k - 1; j >= 0; j--) {
      if (slots[j] >= 0) out.push({ id: this.#ids[slots[j]], score: scores[j] });
    }
    return out;
  }

  #check(vector) {
    if (!vector || vector.length !== this.#dimensions) {
      throw new RangeError(
        `VectorIndex: expected a vector of length ${this.#dimensions}, got ${vector?.length}`,
      );
    }
  }
}
