export default function LoadingButton({
  loading = false,
  loadingLabel = "处理中",
  spinnerClassName = "h-4 w-4",
  className = "",
  disabled = false,
  children,
  ...props
}) {
  const isDisabled = Boolean(disabled) || loading;
  return (
    <button className={className} disabled={isDisabled} {...props}>
      {loading ? (
        <span className="inline-flex items-center gap-2">
          <span
            className={`${spinnerClassName} animate-spin rounded-full border-2 border-white/80 border-t-transparent`}
          />
          {loadingLabel}
        </span>
      ) : (
        children
      )}
    </button>
  );
}
