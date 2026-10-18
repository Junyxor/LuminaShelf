import { Heart } from "lucide-react";
import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";

type Props = {
  cover: ReactNode;
  title: string;
  subtitle: string;
  openLabel: string;
  onOpen: () => void;
  busy?: boolean;
  action?: ReactNode;
  coverProps?: ButtonHTMLAttributes<HTMLButtonElement>;
} & Omit<HTMLAttributes<HTMLElement>, "title">;

/** A cover that opens the book, with its title, author line and one quiet action. */
export default function BookCard({ cover, title, subtitle, openLabel, onOpen, busy, action, coverProps, ...article }: Props) {
  return <article className="shelf-book" {...article}>
    <button className="cover-button" aria-label={openLabel} onClick={onOpen} disabled={busy} {...coverProps}>
      {cover}
      {busy ? <span className="cover-busy">正在打开</span> : null}
    </button>
    <h3>{title}</h3>
    <div className="book-caption"><p>{subtitle}</p>{action}</div>
  </article>;
}

export function FavoriteButton({ active, title, onToggle }: { active: boolean; title: string; onToggle: () => void }) {
  return <button className={`icon-button ${active ? "is-on" : ""}`} aria-label={`${active ? "取消收藏" : "收藏"} ${title}`} aria-pressed={active} onClick={onToggle}>
    <Heart size={18} strokeWidth={1.8} fill={active ? "currentColor" : "none"} />
  </button>;
}

export function SkeletonGrid({ label, count = 8 }: { label: string; count?: number }) {
  return <div className="shelf-grid skeleton-grid" role="status">
    <span className="visually-hidden">{label}</span>
    {Array.from({ length: count }, (_, index) => <div key={index} aria-hidden="true"><div className="skeleton-cover" /><div className="skeleton-line" /><div className="skeleton-line" /></div>)}
  </div>;
}
