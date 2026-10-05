import type { Backend } from '../backend/backend';
import { t } from '../i18n/index.svelte';
import type { CommandError } from '../types';
import { History } from './history.svelte';
import { emptyProfile, withDefaults, type Block, type Profile } from './profile';

/** Delay before the edited profile goes to the shell for the canvas's data. */
export const EDITOR_PROFILE_DEBOUNCE_MS = 250;

const isCommandError = (e: unknown): e is CommandError => typeof e === 'object' && e !== null && typeof (e as CommandError).key === 'string';
const asCommandError = (e: unknown): CommandError => (isCommandError(e) ? { key: e.key, detail: e.detail ?? null } : { key: 'editor.error.io', detail: String(e) });

/**
 * The profile open in the overlay editor: its edits with undo and redo, the selection, the
 * clipboard and the file actions. Every change tells the shell whether there are unsaved changes
 * (`overlayEditorDirty`) and, debounced, which profile the canvas shows (`overlayEditorProfile`).
 * Actions resolve to false on failure and leave the reason in `error`.
 */
export class EditorStore {
  /** Null for a profile never saved. */
  profileId = $state<string | null>(null);
  builtin = $state(false);
  profile = $state.raw<Profile>(emptyProfile(''));
  /** The profile as last loaded or saved, as JSON. */
  saved = $state('');
  dirty = $state(false);
  selection = $state.raw<ReadonlySet<string>>(new Set());
  clipboard = $state.raw<Block[]>([]);
  error = $state.raw<CommandError | null>(null);
  readonly history: History<Profile>;
  #backend: Backend;
  #sentDirty = false;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(backend: Backend) {
    this.#backend = backend;
    this.history = new History(this.profile);
    this.newProfile();
  }

  /** The selected blocks, in profile order. */
  get selected(): Block[] {
    return this.profile.blocks.filter((b) => this.selection.has(b.id));
  }

  /** Starts a new, unsaved, empty profile, named in the current language (call it again once the settings are known). */
  newProfile(): void {
    const profile = emptyProfile(t('editor.newProfileName'));
    this.#open(null, false, profile, JSON.stringify(profile));
  }

  async load(id: string): Promise<boolean> {
    return this.#run(async () => {
      const loaded = await this.#backend.overlayLoadProfile(id);
      const profile = withDefaults(JSON.parse(loaded.json));
      this.#open(loaded.id, loaded.builtin, profile, JSON.stringify(profile));
    });
  }

  /** Records an edit as one undo step (or, inside `begin`/`commit`, as part of the gesture). Ignored on a built-in. */
  apply(next: Profile): void {
    if (this.builtin) return;
    this.history.push(next);
    this.#show(next);
  }

  /** Opens a gesture (a drag): the edits until `commit` are one undo step. */
  begin(): void {
    this.history.begin();
  }

  commit(): void {
    this.history.commit();
  }

  undo(): void {
    this.history.commit();
    const previous = this.history.undo();
    if (previous !== undefined) this.#show(previous);
  }

  redo(): void {
    this.history.commit();
    const next = this.history.redo();
    if (next !== undefined) this.#show(next);
  }

  select(ids: Iterable<string>): void {
    this.selection = new Set(ids);
  }

  copy(): void {
    this.clipboard = structuredClone(this.selected);
  }

  /** Renames the profile as an edit; saving writes it. */
  rename(name: string): void {
    this.apply({ ...this.profile, name });
  }

  async save(): Promise<boolean> {
    if (this.builtin) {
      this.error = { key: 'editor.error.readOnly', detail: null };
      return false;
    }
    return this.#run(async () => {
      const json = JSON.stringify(this.profile);
      this.profileId = await this.#backend.overlaySaveProfile(this.profileId, json);
      this.saved = json;
      this.#syncDirty();
    });
  }

  /** Saves a copy under a new id (the shell makes the name unique) and opens it. */
  async saveAs(name: string): Promise<boolean> {
    // A built-in goes through «Duplicate», which binds its sensors to this PC by role (DD10).
    if (this.builtin) {
      if (!(await this.duplicate())) return false;
      this.rename(name);
      return this.save();
    }
    let id = '';
    const ok = await this.#run(async () => {
      id = await this.#backend.overlaySaveProfile(null, JSON.stringify({ ...this.profile, name }));
    });
    return ok && this.load(id);
  }

  /** A user copy of the saved profile, opened in its place; a built-in is bound to this PC by role. */
  async duplicate(): Promise<boolean> {
    const from = this.profileId;
    if (from === null) return this.saveAs(this.profile.name);
    let id = '';
    const ok = await this.#run(async () => {
      id = await this.#backend.overlayDuplicateProfile(from);
    });
    return ok && this.load(id);
  }

  /** Deletes the saved profile and starts a new one. */
  async remove(): Promise<boolean> {
    const id = this.profileId;
    if (this.builtin) {
      this.error = { key: 'editor.error.readOnly', detail: null };
      return false;
    }
    if (id === null) {
      this.newProfile();
      return true;
    }
    return this.#run(async () => {
      await this.#backend.overlayDeleteProfile(id);
      this.newProfile();
    });
  }

  /** Imports a file and opens it; true without opening anything when the dialog is cancelled. */
  async importFile(): Promise<boolean> {
    let id: string | null = null;
    const ok = await this.#run(async () => {
      id = await this.#backend.overlayImportProfile();
    });
    return ok && (id === null || this.load(id));
  }

  /** Exports the saved profile; false also when the dialog is cancelled or the profile was never saved. */
  async exportFile(): Promise<boolean> {
    const id = this.profileId;
    if (id === null) return false;
    let exported = false;
    const ok = await this.#run(async () => {
      exported = await this.#backend.overlayExportProfile(id);
    });
    return ok && exported;
  }

  /** The editor is closing: the canvas's data stops. */
  close(): void {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    void this.#backend.overlayEditorProfile(null).catch(() => {});
  }

  async #run(action: () => Promise<void>): Promise<boolean> {
    try {
      await action();
      this.error = null;
      return true;
    } catch (e) {
      this.error = asCommandError(e);
      return false;
    }
  }

  #open(id: string | null, builtin: boolean, profile: Profile, saved: string) {
    this.profileId = id;
    this.builtin = builtin;
    this.saved = saved;
    this.selection = new Set();
    this.history.reset(profile);
    this.#show(profile);
  }

  /** Shows `profile` and tells the shell. */
  #show(profile: Profile) {
    this.profile = profile;
    const ids = new Set(profile.blocks.map((b) => b.id));
    if ([...this.selection].some((id) => !ids.has(id))) this.selection = new Set([...this.selection].filter((id) => ids.has(id)));
    this.#syncDirty();
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      void this.#backend.overlayEditorProfile(JSON.stringify(this.profile)).catch(() => {});
    }, EDITOR_PROFILE_DEBOUNCE_MS);
  }

  #syncDirty() {
    this.dirty = JSON.stringify(this.profile) !== this.saved;
    if (this.dirty === this.#sentDirty) return;
    const sent = this.dirty;
    this.#sentDirty = sent;
    void this.#backend.overlayEditorDirty(sent).catch(() => {
      // Not delivered: the next change sends the state again.
      if (this.#sentDirty === sent) this.#sentDirty = !sent;
    });
  }
}
