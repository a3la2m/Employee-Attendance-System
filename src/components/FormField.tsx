import type { ReactNode } from "react";

/**
 * Design System موحّد للنماذج (RTL).
 * مكوّنات خفيفة تُغلّف كلاسات CSS المشتركة (index.css) لضمان تناسق
 * العناوين والحقول والقوائم والنصوص التوضيحية في كامل النظام.
 *
 * الاستخدام:
 *   <FormField label="اسم الموظف" helper="كما في الهوية">
 *     <FormInput value={name} onChange={...} />
 *   </FormField>
 */

export function FormLabel({
  children,
  htmlFor,
  required,
}: {
  children: ReactNode;
  htmlFor?: string;
  required?: boolean;
}) {
  return (
    <label htmlFor={htmlFor} className="label">
      {children}
      {required && <span className="text-rose-600 mr-1">*</span>}
    </label>
  );
}

export function FormInput(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={`input ${props.className ?? ""}`.trim()} />;
}

export function FormSelect(props: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return <select {...props} className={`input ${props.className ?? ""}`.trim()} />;
}

export function FormTextarea(props: React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...props} className={`input ${props.className ?? ""}`.trim()} />;
}

export function FormHelperText({ children }: { children: ReactNode }) {
  return <span className="helper">{children}</span>;
}

export function FormError({ children }: { children: ReactNode }) {
  return <span className="error-text">{children}</span>;
}

export function FormField({
  label,
  htmlFor,
  required,
  helper,
  error,
  children,
}: {
  label?: ReactNode;
  htmlFor?: string;
  required?: boolean;
  helper?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="field">
      {label !== undefined && (
        <FormLabel htmlFor={htmlFor} required={required}>
          {label}
        </FormLabel>
      )}
      {children}
      {error ? <FormError>{error}</FormError> : helper ? <FormHelperText>{helper}</FormHelperText> : null}
    </div>
  );
}

export default FormField;