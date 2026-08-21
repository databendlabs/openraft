(ns jepsen.openraft.await
  (:require [jepsen.openraft.interruption :as interruption]
            [jepsen.util :as util]))

(def ^:private retry-kind ::retry)
(def ^:private condition-timeout-kind ::condition-timeout)
(def ^:private escaped-exception (Object.))

(defn retry!
  "Signals that an owned SUT condition is not satisfied yet."
  [condition details]
  (throw (ex-info "OpenRaft SUT condition is not satisfied yet"
                  (assoc details
                         :kind retry-kind
                         :condition condition))))

(defn condition-timeout?
  "True when e is a timeout produced while waiting for condition."
  [e condition]
  (let [data (ex-data e)]
    (and (= condition-timeout-kind (:kind data))
         (= condition (:condition data)))))

(defn- retry-exception? [e condition]
  (let [data (ex-data e)]
    (and (= retry-kind (:kind data))
         (= condition (:condition data)))))

(defn- propagate! [e]
  (when (interruption/interruption? e)
    (.interrupt (Thread/currentThread)))
  (throw e))

(defn until!
  "Polls until f returns, retrying only retry! for the owned condition.

  Unknown exceptions return through an opaque value so Jepsen's broad
  await-fn catch cannot retry them."
  [condition f opts]
  (let [result
        (try
          (util/await-fn
           #(try
              (f)
              (catch Exception e
                (if (retry-exception? e condition)
                  (throw e)
                  [escaped-exception e])))
           opts)
          (catch Exception e
            (cond
              (interruption/interruption? e)
              (propagate! e)

              (and (= :timeout (:type (ex-data e)))
                   (retry-exception? (ex-cause e) condition))
              (throw (ex-info "Timed out waiting for an OpenRaft SUT condition"
                              {:kind condition-timeout-kind
                               :condition condition}
                              e))

              :else
              (throw e))))]
    (if (and (vector? result)
             (identical? escaped-exception (first result)))
      (propagate! (second result))
      result)))
