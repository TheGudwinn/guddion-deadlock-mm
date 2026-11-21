import { Component, type ReactNode } from "react";
import { withTranslation, type WithTranslation } from "react-i18next";

type Props = WithTranslation & { children: ReactNode };

/** Keeps a crashing screen from taking the whole launcher down: shows a short message and a way back. */
class Boundary extends Component<Props, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.error("screen crashed", error);
  }

  render() {
    const { t } = this.props;
    if (!this.state.failed) return this.props.children;
    return (
      <section className="view">
        <div className="notice" role="alert">
          <b>{t("crash.title")}</b>
          <span>{t("crash.text")}</span>
          <div className="row">
            <button className="btn" onClick={() => this.setState({ failed: false })}>
              {t("crash.retry")}
            </button>
          </div>
        </div>
      </section>
    );
  }
}

export default withTranslation()(Boundary);
