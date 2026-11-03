(ns jepsen.openraft.scenario.leader-outside-joint-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.scenario.leader-outside-joint :as scenario]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- step [f value]
  {:process :nemesis :type :info :f f :value value})

(def ^:private covered
  [(step :prepare-joint {:status :prepared
                         :committed-voter-configs [#{"n1" "n2" "n3"}
                                                   #{"n1" "n3"}]})
   (step :transfer-to-old-half {:status :transferred :leader "n2"})])

(def ^:private recovered
  (step :check-leader-outside-joint-recovery
        {:status :recovered
         :elapsed-ms 0
         :voters #{"n3"}
         :fresh "after-change"
         :sentinel "before-change"}))

(def ^:private change-response
  {:log_id {:leader_id {:term 2 :node_id "n2"} :index 8}
   :data {:value nil}
   :membership {:configs [["n3"]]
                :nodes {:n3 {:raft_addr "n3:22001" :data "n3:21001"}}}})

(deftest accepts-a-completed-change-that-recovers
  (let [change {:status :changed}
        history (conj covered (step :change-from-old-half change) recovered)
        invocations-and-completions (mapcat #(vector (assoc % :value nil) %) history)
        recovery (:value recovered)]
    (doseq [operations [history invocations-and-completions]]
      (let [result (scenario/verdict test-config operations)]
        (is (= {:valid? true
                :scenario-covered? true
                :changed? true
                :change change
                :recovered? true
                :recovery recovery}
               result))))))

(deftest enforces-the-recovery-deadline
  (doseq [[elapsed-ms expected] [[0 true] [60000 true] [60001 false] [-1 false] [nil false]]]
    (let [recovery (if (some? elapsed-ms)
                     (assoc-in recovered [:value :elapsed-ms] elapsed-ms)
                     (update recovered :value dissoc :elapsed-ms))
          history (conj covered
                        (step :change-from-old-half {:status :changed})
                        recovery)
          result (scenario/verdict test-config history)
          valid? (:valid? result)
          recovered? (:recovered? result)]
      (testing (str "elapsed-ms=" elapsed-ms)
        (is (= expected valid?))
        (is (= expected recovered?))))))

(deftest rejects-any-other-answer-to-the-change
  (doseq [change [{:status :refused :error-variant :LearnerNotFound}
                  {:status :refused :error-variant :InProgress}
                  {:status :refused :error-variant :UncommittedLeaderLog}
                  {:status :not-leader}
                  nil]]
    (testing (pr-str change)
      (let [result (scenario/verdict
                    test-config
                    (conj covered
                          (step :change-from-old-half change)
                          recovered))]
        (is (false? (:valid? result)))
        (is (false? (:changed? result)))))))

(deftest rejects-a-discarded-change-even-after-recovery
  (let [change {:status :indeterminate :error-variant :LogEntryDiscarded :attempts 1}
        history (conj covered (step :change-from-old-half change) recovered)
        recovery (:value recovered)
        result (scenario/verdict test-config history)]
    (is (= {:valid? false
            :scenario-covered? true
            :changed? false
            :change change
            :recovered? true
            :recovery recovery}
           result))))

(deftest rejects-an-unanswered-change-or-a-stuck-cluster
  (testing "a change the leader never answers, as when its core panics"
    (let [result (scenario/verdict
                  test-config
                  (conj covered
                        (step :change-from-old-half
                              {:status :indeterminate :kind :transport-error})
                        recovered))]
      (is (false? (:valid? result)))
      (is (true? (:scenario-covered? result)))))

  (testing "a completed change after which the cluster never recovers"
    (is (false? (:valid? (scenario/verdict
                          test-config
                          (conj covered
                                (step :change-from-old-half
                                      {:status :changed})
                                (step :check-leader-outside-joint-recovery
                                      {:status :timeout})))))))

  (testing "a transfer that never reached the old half is not coverage"
    (let [result (scenario/verdict
                  test-config
                  [(first covered)
                   (step :transfer-to-old-half {:status :transferred
                                                :leader "n1"})
                   (step :change-from-old-half {:status :changed})
                   recovered])
          covered? (:scenario-covered? result)
          valid? (:valid? result)]
      (is (false? covered?))
      (is (false? valid?)))))

(deftest rejects-the-wrong-membership-or-lost-data
  (doseq [recovery [(assoc-in recovered [:value :voters] #{"n1" "n2" "n3"})
                    (assoc-in recovered [:value :fresh] nil)
                    (assoc-in recovered [:value :sentinel] nil)]]
    (let [result (scenario/verdict
                  test-config
                  (conj covered
                        (step :change-from-old-half {:status :changed})
                        recovery))]
      (is (false? (:valid? result)))
      (is (false? (:recovered? result))))))

(def ^:private joint-voter-configs [#{"n1" "n2" "n3"} #{"n1" "n3"}])

(defn- led-by
  "A membership status whose leader has last log index 4 and has seen X (n2)
  acknowledge x-matched."
  [leader x-matched]
  {:leader leader
   :effective-voter-configs joint-voter-configs
   :metrics {leader {:last_log_index 4
                     :replication {:n2 (when x-matched {:index x-matched})}}}})

(defn- run-transfer!
  "Runs transfer! against one status per poll, the last one repeated, and
  returns its result or exception with the transfer requests it sent."
  [statuses redefs]
  (let [statuses (atom statuses)
        sent (atom [])
        until! await/until!]
    (with-redefs-fn
      (merge {#'cluster/membership-status (fn [_test]
                                            (let [status (first @statuses)]
                                              (when (next @statuses)
                                                (swap! statuses rest))
                                              status))
              #'http/transfer-leader! (fn [endpoint node-id]
                                        (swap! sent conj [endpoint node-id]))
              #'await/until! (fn [condition f opts]
                               (until! condition f (assoc opts :retry-interval 0)))}
             redefs)
      #(let [outcome (try
                       {:result (#'scenario/transfer! test-config)}
                       (catch Exception e
                         {:error e}))]
         (assoc outcome :sent @sent)))))

(deftest transfer-waits-for-x-to-catch-up
  (let [{:keys [result sent]}
        (run-transfer! [nil (led-by "n1" 3) (led-by "n1" 4) (led-by "n2" nil)]
                       {#'scenario/transfer-retry-ms 0})]
    (is (= [["n1:21001" "n2"]] sent))
    (is (= {:status :transferred
            :leader "n2"
            :effective-voter-configs joint-voter-configs
            :transfer-requests ["n1"]}
           result))))

(deftest transfer-asks-whichever-node-leads-again
  (testing "the node that leads after a refused transfer is asked"
    (let [{:keys [result sent]}
          (run-transfer! [(led-by "n1" 4) (led-by "n3" 4) (led-by "n2" nil)]
                         {#'scenario/transfer-retry-ms 0})]
      (is (= [["n1:21001" "n2"] ["n3:21001" "n2"]] sent))
      (is (= ["n1" "n3"] (:transfer-requests result)))))

  (testing "a request is not repeated within the retry interval"
    (let [{:keys [result sent]}
          (run-transfer! [(led-by "n1" 4) (led-by "n1" 4) (led-by "n2" nil)]
                         {#'scenario/transfer-retry-ms 60000})]
      (is (= [["n1:21001" "n2"]] sent))
      (is (= ["n1"] (:transfer-requests result))))))

(deftest transfer-stops-at-the-setup-deadline
  (let [{:keys [error sent]}
        (run-transfer! [(led-by "n1" 3)] {#'scenario/transfer-timeout-ms 0})]
    (is (await/condition-timeout? error :old-half-leader))
    (is (= [] sent))))

(deftest classifies-the-change-response
  (testing "a completed change"
    (with-redefs [http/change-membership! (fn [& _] change-response)]
      (let [result (#'scenario/change! test-config)]
        (is (= {:status :changed :response change-response :attempts 1} result)))))

  (doseq [[label thrown expected]
          [["a refusal"
            (ex-info "refused" {:kind :openraft-error
                                :error {:ChangeMembershipError
                                        {:LearnerNotFound {}}}})
            {:status :refused :error-variant :LearnerNotFound :attempts 1}]
           ["a lost leadership"
            (ex-info "forward" {:kind :openraft-error
                                :error {:ForwardToLeader {:leader_id "n1"}}})
            {:status :not-leader :attempts 1}]
           ["a discarded membership entry"
            (ex-info "discarded" {:kind :openraft-error
                                  :error {:LogEntryDiscarded {:leader_id "n3"}}})
            {:status :indeterminate :error-variant :LogEntryDiscarded :attempts 1}]
           ["no answer"
            (ex-info "reset" {:kind :transport-error})
            {:status :indeterminate :kind :transport-error :attempts 1}]]]
    (testing label
      (let [attempts (atom 0)]
        (with-redefs [http/change-membership! (fn [endpoint node-ids]
                                                (swap! attempts inc)
                                                (is (= "n2:21001" endpoint))
                                                (is (= ["n3"] node-ids))
                                                (throw thrown))]
          (let [result (#'scenario/change! test-config)]
            (is (= expected result))
            (is (= 1 @attempts)))))))

  (testing "a refusal that only means X is not ready yet is retried"
    (let [answers (atom [:InProgress :UncommittedLeaderLog nil])]
      (with-redefs [http/change-membership!
                    (fn [& _]
                      (let [variant (first @answers)]
                        (swap! answers rest)
                        (if variant
                          (throw (ex-info "refused"
                                          {:kind :openraft-error
                                           :error {:ChangeMembershipError
                                                   {variant {}}}}))
                          change-response)))]
        (let [result (#'scenario/change! test-config)]
          (is (= {:status :changed :response change-response :attempts 3} result))))))

  (testing "an unknown error is a Harness failure"
    (with-redefs [http/change-membership! (fn [& _]
                                            (throw (IllegalStateException. "bug")))]
      (is (thrown? IllegalStateException (#'scenario/change! test-config))))))

(deftest change-stops-at-the-refusal-deadline
  (let [error {:kind :openraft-error
               :error {:ChangeMembershipError {:InProgress {}}}}
        requests (atom [])
        result (with-redefs-fn
                 {#'scenario/change-timeout-ms 0
                  #'http/change-membership! (fn [endpoint voters]
                                              (swap! requests conj [endpoint voters])
                                              (throw (ex-info "refused" error)))}
                 #(#'scenario/change! test-config))]
    (is (= {:status :refused :error-variant :InProgress :attempts 1} result))
    (is (= [["n2:21001" ["n3"]]] @requests))))

(deftest recovery-waits-for-the-requested-membership
  (let [bridge {:leader "n3"
                :stable? false
                :effective-voter-configs [#{"n1" "n3"} #{"n3"}]}
        wrong-membership {:leader "n1"
                          :stable? true
                          :voters #{"n1" "n2" "n3"}}
        statuses (atom [bridge wrong-membership (assoc bridge
                                                       :stable? true
                                                       :voters #{"n3"}
                                                       :effective-voter-configs [#{"n3"}])])
        writes (atom [])]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (when (next @statuses)
                        (swap! statuses rest))
                      status))
                  http/change-membership!
                  (fn [& _]
                    (throw (IllegalStateException. "recovery must only observe membership")))
                  http/write! (fn [& args]
                                (swap! writes conj (vec args))
                                {:value "after-change" :version 9})
                  http/linearizable-read! (fn [_ k]
                                            {:value (if (= "leader-outside-joint-fresh" k)
                                                      "after-change"
                                                      "before-change")
                                             :version 9})]
      (let [result (#'scenario/recover! test-config)]
        (is (= :recovered (:status result)))
        (is (= #{"n3"} (:voters result)))
        (is (= "after-change" (:fresh result)))
        (is (= "before-change" (:sentinel result)))
        (is (= [["n3:21001" "leader-outside-joint-fresh" "after-change"]]
               @writes))))))

(deftest recovery-retries-a-modeled-write-error
  (let [status {:leader "n3" :stable? true :voters #{"n3"}}
        writes (atom [])
        until! await/until!]
    (with-redefs [scenario/recovery-ms 1000
                  await/until! (fn [condition f opts]
                                 (until! condition f (assoc opts :retry-interval 0)))
                  cluster/membership-status (constantly status)
                  http/write! (fn [& args]
                                (swap! writes conj (vec args))
                                (when (= 1 (count @writes))
                                  (throw (ex-info "forward"
                                                  {:kind :openraft-error
                                                   :error {:ForwardToLeader {}}}))))
                  http/linearizable-read! (fn [_ key]
                                            {:value (if (= key "leader-outside-joint-fresh")
                                                      "after-change"
                                                      "before-change")})]
      (let [result (#'scenario/recover! test-config)
            observation (dissoc result :elapsed-ms)
            write ["n3:21001" "leader-outside-joint-fresh" "after-change"]]
        (is (= {:status :recovered :leader "n3" :voters #{"n3"}
                :fresh "after-change" :sentinel "before-change"}
               observation))
        (is (= [write write] @writes))))))

(deftest recovery-timeout-records-the-last-request-error
  (doseq [fail-at [:write :read]]
    (let [status {:leader "n3" :stable? true :voters #{"n3"} :metrics :omitted}
          error {:kind :request-timeout :operation fail-at}
          calls (atom [])]
      (with-redefs [scenario/recovery-ms 0
                    cluster/membership-status (constantly status)
                    http/write! (fn [& _]
                                  (swap! calls conj :write)
                                  (when (= :write fail-at)
                                    (throw (ex-info "write timeout" error))))
                    http/linearizable-read! (fn [& _]
                                              (swap! calls conj :read)
                                              (throw (ex-info "read timeout" error)))]
        (let [result (#'scenario/recover! test-config)
              observation (dissoc result :elapsed-ms)
              elapsed-ms (:elapsed-ms result)
              membership (dissoc status :metrics)
              expected-calls (if (= :write fail-at) [:write] [:write :read])]
          (is (= {:status :timeout :membership membership :error error}
                 observation))
          (is (<= 0 elapsed-ms))
          (is (= expected-calls @calls)))))))

(deftest recovery-propagates-unexpected-errors
  (doseq [error [(IllegalStateException. "bug")
                 (ex-info "unexpected membership error"
                          {:kind :openraft-error
                           :error {:ChangeMembershipError {:InProgress {}}}})]]
    (with-redefs [scenario/recovery-ms 0
                  cluster/membership-status (constantly {:leader "n3" :stable? true :voters #{"n3"}})
                  http/write! (fn [& _] (throw error))]
      (let [result (try (#'scenario/recover! test-config)
                        (catch Exception e e))]
        (is (identical? error result))))))
