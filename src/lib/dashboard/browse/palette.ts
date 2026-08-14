// Stable on-brand colour for studio + tag cards.
//
// The design calls for a hue "from the brand palette, NOT random". We hash the
// entity's key (Stash id) to a fixed index into a curated set of design-token
// colours, so the same studio/tag always gets the same colour across renders.

const PALETTE = [
  "#EF6B7A", // coral-500 (brand)
  "#6FA5FF", // blue-500
  "#4ADE80", // green-500
  "#FBB454", // amber-500
  "#F2E8D4", // bone-400
  "#FAA8B1", // coral-300
  "#F58D8D", // red-400
];

export function colorForKey(key: string): string {
  let h = 0;
  for (let i = 0; i < key.length; i++) {
    h = (h * 31 + key.charCodeAt(i)) | 0;
  }
  return PALETTE[Math.abs(h) % PALETTE.length];
}
