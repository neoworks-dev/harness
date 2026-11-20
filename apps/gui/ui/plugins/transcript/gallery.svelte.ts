// The open gallery: the pictures it walks and the one shown. Empty while it is closed.
import { stepIndex, type Picture } from "./gallery.ts";

class Gallery {
  pictures = $state.raw<readonly Picture[]>([]);
  index = $state(0);

  get isOpen(): boolean {
    return this.pictures.length > 0;
  }

  get current(): Picture | undefined {
    return this.pictures[this.index];
  }

  open(pictures: readonly Picture[], index: number): void {
    this.pictures = pictures;
    this.index = index;
  }

  close(): void {
    this.pictures = [];
    this.index = 0;
  }

  step(by: number): void {
    this.index = stepIndex(this.index, this.pictures.length, by);
  }
}

export const gallery = new Gallery();
