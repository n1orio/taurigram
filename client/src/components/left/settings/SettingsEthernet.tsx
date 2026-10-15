import type { FC } from '../../../lib/teact/teact';
import { memo, useState } from '../../../lib/teact/teact';
import { getActions, getGlobal } from '../../../global';

import { SettingsScreens } from '../../../types';

import buildClassName from '../../../util/buildClassName';
import { getEthernetString } from '../../../util/ethernetLang';
import { loadStoredSession } from '../../../util/sessions';

import useHistoryBack from '../../../hooks/useHistoryBack';
import useLang from '../../../hooks/useLang';
import useLastCallback from '../../../hooks/useLastCallback';

import Island from '../../gili/layout/Island';
import ConfirmDialog from '../../ui/ConfirmDialog';
import ListItem from '../../ui/ListItem';

import styles from './SettingsEthernet.module.scss';

import experimentIcon from '../../../assets/ethernet-experiment.svg';
import networkIcon from '../../../assets/ethernet-network.svg';
import pluginsIcon from '../../../assets/ethernet-plugins.svg';
import sparksIcon from '../../../assets/ethernet-sparks.svg';
import themesIcon from '../../../assets/ethernet-themes.svg';

function renderIcon(src: string) {
  return <img src={src} alt="" className={buildClassName('ListItem-main-icon', styles.itemIcon)} draggable={false} />;
}

type OwnProps = {
  isActive?: boolean;
  onReset: () => void;
};

const SettingsEthernet: FC<OwnProps> = ({ isActive, onReset }) => {
  const { openSettingsScreen, openChatByUsername, showNotification } = getActions();
  const lang = useLang();

  useHistoryBack({
    isActive,
    onBack: onReset,
  });

  const openFunctions = useLastCallback(() => openSettingsScreen({ screen: SettingsScreens.EthernetFunctions }));
  const openNetwork = useLastCallback(() => openSettingsScreen({ screen: SettingsScreens.EthernetNetwork }));
  const openThemes = useLastCallback(() => openSettingsScreen({ screen: SettingsScreens.EthernetThemes }));
  const openPlugins = useLastCallback(() => openSettingsScreen({ screen: SettingsScreens.EthernetPlugins }));
  const openExperimental = useLastCallback(() => openSettingsScreen({ screen: SettingsScreens.Experimental }));
  const openEthernetChannel = useLastCallback(() => openChatByUsername({ username: 'niori0' }));

  const [isExportConfirmOpen, setIsExportConfirmOpen] = useState(false);
  const canExportTdata = Boolean(window.ethernetDesktop?.exportTdata);

  const handleExportTdata = useLastCallback(async () => {
    setIsExportConfirmOpen(false);
    const session = loadStoredSession();
    const userId = getGlobal().currentUserId;
    if (!session || !userId || session.isTest) {
      showNotification({ message: getEthernetString(lang, 'EthernetExportTdataUnavailable') });
      return;
    }

    try {
      const path = await window.ethernetDesktop!.exportTdata!({
        userId,
        mainDcId: session.mainDcId,
        keys: session.keys,
      });
      if (path) {
        showNotification({ message: `${getEthernetString(lang, 'EthernetExportTdataDone')} ${path}` });
      }
    } catch (err) {
      showNotification({ message: String(err) });
    }
  });

  return (
    <div className="settings-content custom-scroll">
      <Island>
        <ListItem
          multiline
          narrow
          leftElement={renderIcon(sparksIcon)}
          onClick={openFunctions}
        >
          <span className="title">{getEthernetString(lang, 'EthernetFunctions')}</span>
          <span className="subtitle">{getEthernetString(lang, 'EthernetFunctionsDesc')}</span>
        </ListItem>
        <ListItem
          multiline
          narrow
          leftElement={renderIcon(networkIcon)}
          onClick={openNetwork}
        >
          <span className="title">{getEthernetString(lang, 'EthernetNetwork')}</span>
          <span className="subtitle">{getEthernetString(lang, 'EthernetNetworkDesc')}</span>
        </ListItem>
        <ListItem
          multiline
          narrow
          leftElement={renderIcon(themesIcon)}
          onClick={openThemes}
        >
          <span className="title">{getEthernetString(lang, 'EthernetThemes')}</span>
          <span className="subtitle">{getEthernetString(lang, 'EthernetThemesDesc')}</span>
        </ListItem>
        <ListItem
          multiline
          narrow
          leftElement={renderIcon(pluginsIcon)}
          onClick={openPlugins}
        >
          <span className="title">{getEthernetString(lang, 'EthernetPlugins')}</span>
          <span className="subtitle">{getEthernetString(lang, 'EthernetPluginsDesc')}</span>
        </ListItem>
        <ListItem
          multiline
          narrow
          leftElement={renderIcon(experimentIcon)}
          onClick={openExperimental}
        >
          <span className="title">{getEthernetString(lang, 'EthernetExperimental')}</span>
          <span className="subtitle">{getEthernetString(lang, 'EthernetExperimentalDesc')}</span>
        </ListItem>
        {canExportTdata && (
          <ListItem
            multiline
            narrow
            icon="download"
            onClick={() => setIsExportConfirmOpen(true)}
          >
            <span className="title">{getEthernetString(lang, 'EthernetExportTdata')}</span>
            <span className="subtitle">{getEthernetString(lang, 'EthernetExportTdataDesc')}</span>
          </ListItem>
        )}
      </Island>

      <div className={styles.authorSection}>
        <button
          type="button"
          className={styles.ethernetButton}
          onClick={openEthernetChannel}
          title="@niori0"
        >
          {getEthernetString(lang, 'EthernetForkAuthorChannel')}
        </button>
      </div>

      <ConfirmDialog
        isOpen={isExportConfirmOpen}
        onClose={() => setIsExportConfirmOpen(false)}
        title={getEthernetString(lang, 'EthernetExportTdata')}
        text={getEthernetString(lang, 'EthernetExportTdataConfirm')}
        confirmLabel={getEthernetString(lang, 'EthernetExportTdataButton')}
        confirmHandler={handleExportTdata}
      />
    </div>
  );
};

export default memo(SettingsEthernet);
