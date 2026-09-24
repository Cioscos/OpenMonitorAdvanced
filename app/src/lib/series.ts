/** Fixed-capacity ring buffer of samples; NaN marks a missing value. */
export class SeriesBuffer {
  readonly capacity: number;
  #data: Float64Array;
  #start = 0;
  #length = 0;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.#data = new Float64Array(capacity);
  }

  get length(): number {
    return this.#length;
  }

  push(value: number | null): void {
    const v = value ?? NaN;
    if (this.#length < this.capacity) {
      this.#data[(this.#start + this.#length) % this.capacity] = v;
      this.#length++;
    } else {
      this.#data[this.#start] = v;
      this.#start = (this.#start + 1) % this.capacity;
    }
  }

  toArray(): number[] {
    return Array.from({ length: this.#length }, (_, i) => this.#data[(this.#start + i) % this.capacity]);
  }

  clear(): void {
    this.#start = 0;
    this.#length = 0;
  }
}
