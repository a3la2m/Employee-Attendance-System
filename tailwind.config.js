/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ["Tajawal", "system-ui", "sans-serif"],
      },
      colors: {
        primary: {
          50: "#E1F5FE",
          100: "#B3E5FC",
          200: "#81D4FA",
          300: "#4FC3F7",
          400: "#29B6F6",
          500: "#039BE5",
          600: "#0277BD",
          700: "#0277BD",
          800: "#01579B",
          900: "#004C8C",
        },
        success: {
          50: "#E8F5E9",
          100: "#C8E6C9",
          200: "#A5D6A7",
          300: "#81C784",
          400: "#66BB6A",
          500: "#43A047",
          600: "#2E7D32",
          700: "#2E7D32",
          800: "#1B5E20",
          900: "#1B5E20",
        },
      },
    },
  },
  plugins: [],
};