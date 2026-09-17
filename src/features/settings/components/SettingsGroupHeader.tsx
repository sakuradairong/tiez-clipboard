import type { ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

interface SettingsGroupHeaderProps {
  title: string;
  collapsed: boolean;
  onToggle: () => void;
  description?: string;
  /** Extra content beside the title (status dot, badge, etc.) */
  titleExtra?: ReactNode;
}

/**
 * Accessible settings accordion header: keyboard-focusable with aria-expanded.
 */
const SettingsGroupHeader = ({
  title,
  collapsed,
  onToggle,
  description,
  titleExtra
}: SettingsGroupHeaderProps) => (
  <button
    type="button"
    className="group-header"
    onClick={onToggle}
    aria-expanded={!collapsed}
  >
    <div className="group-header-text">
      <div className="group-header-title-row">
        <h3 style={{ margin: 0 }}>{title}</h3>
        {titleExtra}
      </div>
      {description ? (
        <div className="settings-subpage-note">{description}</div>
      ) : null}
    </div>
    {collapsed ? (
      <ChevronRight size={16} aria-hidden="true" />
    ) : (
      <ChevronDown size={16} aria-hidden="true" />
    )}
  </button>
);

export default SettingsGroupHeader;
