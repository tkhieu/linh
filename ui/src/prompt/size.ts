export const PROMPT_WIDTH = 360
const MONITOR_MARGIN = 48

/** CSS pixels throughout; monitor work area must already be converted to logical pixels. */
export function promptSize(contentHeight: number, workAreaHeight: number, listHeight: number) {
  const availableHeight = Math.max(1, Math.floor(workAreaHeight - MONITOR_MARGIN))
  const height = Math.min(Math.ceil(contentHeight), availableHeight)
  return {
    width: PROMPT_WIDTH,
    height,
    // Only the choices scroll when the full content exceeds the work area.
    maxListHeight: contentHeight > availableHeight
      ? Math.max(0, Math.floor(listHeight - (contentHeight - availableHeight)))
      : null,
  }
}
